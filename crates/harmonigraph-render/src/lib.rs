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

/// A name's shadow: its ink blurred into a cell of an atlas, which the name's
/// own draw multiplies the frame by.
mod shadow;

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
    /// A node's own radius on this pane, in POINTS.
    ///
    /// A lattice name paints no rim; what holds a halo off it is the shadow it
    /// casts (`fs_shadow_box`), and that is dialled in node radii like every
    /// other shadow in the picture. The glyph pass has no node under a letter
    /// to measure one against, so the conversion is made here, where the
    /// pane's own height and the camera both are.
    ///
    /// One number for the pane, as the size the names are typeset at is: an
    /// off-sheet node draws smaller and its name with it, and its shadow is
    /// still the home sheet's width.
    pub node_points: f32,
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
/// the single-attachment one the parity test's reference path uses; the
/// `glow` four are the glow's own pass — a billboard and the light it lays
/// down, once for the nodes and once for the resting markers; and the `ink`
/// four are the strip the nodes' light is coloured out of, read and then
/// blurred ahead of it (see [`InkStrip`]).
#[cfg(any(test, feature = "hot-reload"))]
const REQUIRED_ENTRY_POINTS: &[&str] = &[
    "vs_main",
    "fs_main",
    "fs_main_scene",
    "vs_plus",
    "fs_plus",
    "fs_plus_scene",
    "vs_glow",
    "fs_glow",
    "vs_plus_glow",
    "fs_plus_glow",
    "vs_ink_strip",
    "fs_ink_strip",
    "vs_ink_blur",
    "fs_ink_blur",
];

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
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
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
    /// y: base node radius (world units);
    /// z: how much two nodes' overlapping light adds up (`Scene::glow_meld`),
    /// 1 screening the halos together and 0 leaving an overlap as bright as
    /// the brighter node alone. HERE rather than with the glow's own rows
    /// because blit.wgsl's composite reads it too, and that pass sees only the
    /// head of this buffer (`BlitUniforms`) — a dial two shader modules both
    /// need has to live in the part both can address. Written raw rather than
    /// zeroed with `misc10`: it is a factor on a field that is itself absent
    /// where the glow is off, so there is nothing for a zero to switch off.
    /// w unused — it carried the node style, the paint of a disc at the node's
    /// centre, from when that disc had more than one. A retired slot rather
    /// than a repack, which would renumber the ones around it for nothing.
    misc: [f32; 4],
    /// x: darkest_pitch, y: brightest_pitch (MIDI notes); z: render scale
    /// (the shader converts its screen-pixel AA softness to render
    /// pixels with it); w: bloom strength, which blit.wgsl reads (this
    /// slot is NOT free). The dots style maps a dot's pitch through x/y
    /// to index `pitch_lut`.
    misc2: [f32; 4],
    /// x unused — it carried the radius of a disc at the node's centre, the
    /// one layer that was not a ring. Retired in place, like `misc.w`;
    /// y/z: the
    /// outer octave layer's inner/outer band radii (same units), which the
    /// scene hands over either as z > y or as the empty pair (0, 0) that says
    /// the layer is off — the shader gates the band on z > y rather than
    /// assuming it (`glyph_band`'s two soft edges cross instead of cancelling
    /// at z == y, painting a dot at the node's centre); w: the outer edge of
    /// the outermost RING the node draws (`Scene::rings_outer`), which
    /// `node_rim` and the mark strip stand off, so neither has to know
    /// which layer was the last one on.
    misc3: [f32; 4],
    /// Pitch->color lookup for the dots octave style (see harmonigraph_scene's
    /// `pitch_ramp_lut`), matching the node disc gradient.
    pitch_lut: [[f32; 4]; harmonigraph_scene::PITCH_LUT_N],
    /// x: unused — it carried the solidity of a disc at the node's centre;
    /// y: where the melody/bass strip starts (`Scene::mark_inner`);
    /// z/w unused — they carried the idle marker's radius and style, from
    /// when an unlit node drew a placeholder. Retired in place,
    /// like `misc.x`. (The blit pipeline binds only the head of this
    /// buffer, so trailing fields are safe to add here.)
    misc4: [f32; 4],
    /// x: half a resting marker's arm thickness, as a share of one arm's
    /// length (`Scene::plus_half_width`) — 1 is a filled square;
    /// y: where a marker's arms start to taper, as a share of one arm
    /// (`Scene::plus_taper_start`) — 1 is a square end;
    /// z: the node's ANGULAR padding in quad UV units — the gap between two
    /// neighbouring sectors, wherever sectors are drawn. Its RADIAL counterpart
    /// never arrives: every stand-off that one buys is already spent in the
    /// radii in `misc3` and `misc4.y`, so what stands the marks off the band is
    /// `mark_inner`, not a sum taken over this;
    /// w: how far a melody/bass mark reaches past the band, same units, where
    /// 0 means no marks (so this slot is NOT free — `mark_extension` reads it,
    /// and the octave layer gates the marks on it). Safe to have started a new
    /// slot here, per the note on `misc4`.
    misc5: [f32; 4],
    /// x/y unused — they carried the trail's mark style and strength, from
    /// when a memory was a change to the idle marker rather than a kept note
    /// name. Retired in place.
    /// z: unused; w: the melody/bass marks' shimmer pattern
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
    /// The node glow's four dials. x: how far past a node's outermost drawn
    /// edge its light spreads, in quad UV units (`Scene::glow_reach`); y: how
    /// much light (`Scene::glow_strength`); z: how flat the falloff is across
    /// that span (`Scene::glow_feather`), 0 the exponential heaped on the node
    /// and 1 an even field across it; w: how widely a node's own ink is
    /// averaged into the colour of its light (`Scene::glow_blend`).
    ///
    /// ZEROED WHOLE where the glow does not draw, so `x > 0.0` is the single
    /// test on either side of the boundary — [`LatticeCallback::glow_draws`]
    /// here, and lattice.wgsl throughout. A strength of 0 is a glow that draws
    /// nothing, and the three shapes beside it have nothing to shape.
    misc10: [f32; 4],
    /// The GAP's four dials — one shape, read twice: as the hole a node cuts in
    /// everything drawn behind it (`node_clearing` in lattice.wgsl), and as the
    /// light that same shape dims where it stands over a ring (`glow_standoff`).
    /// x: how far past each ring it reaches, in the same quad UV units
    /// (`Scene::glow_shadow`); y: how much of that is spent fading it back off
    /// (`Scene::glow_shadow_soft`); z: how the fade is skewed across that width
    /// (`Scene::glow_shadow_shape`); w: how much of the LIGHT it takes where it
    /// stands (`Scene::glow_shadow_depth`), off the ground the clearing paints —
    /// what a node's own ink takes of that same field is `misc13` below.
    ///
    /// A row of its own rather than four slots scattered over the two beside
    /// it, because the four are one control: the Shadow bar's two handles, the
    /// curve across them, and the depth the finished coverage is scaled by on
    /// the light's side.
    ///
    /// NOT zeroed with `misc10`, which is where this row parts company with
    /// every other one under the glow: the hole is a shape a node cuts whether
    /// or not there is a light in the picture, so zeroing this with the light
    /// would take every node's knockout off with it — sheets interpenetrating,
    /// and the resting markers uncut — the moment the Reach reached 0. What is
    /// zeroed instead is nothing, `glow_shadow` being its own off switch on both
    /// sides (`glow_shadow()` in lattice.wgsl), and the depth is the light's
    /// (`glow_shade`).
    misc11: [f32; 4],
    /// The node glow's plumbing row, which is not a dial. x: how many rows this
    /// frame's ink strip has (`Scene::glow_rows`) — the one thing
    /// `vs_ink_strip` cannot work out for itself, since it is writing that
    /// texture rather than reading one, and a CAPACITY rather than the instance
    /// count, rows being handed out per node and held for as long as that
    /// node's light lasts. y/z/w unused.
    ///
    /// A row of its own rather than the spare half of one above, because it
    /// answers a different question: everything up there is a setting a person
    /// dialled, and this is how tall a texture the renderer allocated. Zeroed
    /// whole with them, on the same rule.
    misc12: [f32; 4],
    /// The WASH. x: how much of the light a LIT slice of a node washes its own
    /// ink with (`Scene::glow_wash`), where every other piece of the lattice's
    /// ink takes that same field whole and `misc11.w` above is the GROUND's
    /// share of it. y: one quad uv as a world length on the markers' sheet
    /// (`Scene::marker_unit`), which is what the markers' standoff draw
    /// converts between its own world lengths and the uv the Shadow it holds
    /// the light off by is dialled in. z/w unused.
    ///
    /// A row of its own because the wash is not a term of the standoff, close
    /// as it reads to the depth: the Shadow bars shape what the clearing
    /// paints, and this reads the field raw, so a dial sitting among them would
    /// carry the coupling it exists to break. Zeroed whole with `misc10`, on
    /// the same rule — a wash with no light to lay down is a factor on nothing,
    /// and the markers' standoff draw reads its own zero there as an off switch
    /// (`vs_plus_glow`).
    misc13: [f32; 4],
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
    /// apiece, sixteen to a row (see `SPECTRUM_WORDS`). Used by the CPU-side
    /// gate; the adjacent color copy is what the shader paints.
    ///
    /// In the uniform buffer rather than a texture, and the two copies are
    /// 7.6 KB together.
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
    /// The same buckets mapped through the volume-color dB window. The ring
    /// shader uses these for color while `spectrum` remains the analyzer copy
    /// used by the CPU gate.
    spectrum_color: [[u32; 4]; SPECTRUM_WORDS],
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
    /// Billboard size factor: 1 on the home sheet, smaller with every step off
    /// it (`NodeInstance::scale`).
    scale: f32,
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
    /// The node's own light: x how bright it is, y which ROW of the ink strip
    /// keeps its colour, z how much of this frame's reading the two of them
    /// take, w how much of a MARK the light still has the node wearing
    /// (`harmonigraph_scene::GlowStep`, filled in by the shell's
    /// `panes::glow_fade`).
    ///
    /// All four are the glow's and nothing else reads them. The level is a
    /// CARRIED one and not the largest envelope on the node, which is the whole
    /// point of it: it can be above zero on a node whose every layer has gone
    /// silent, and such a node is shipped for exactly that reason (see the cull
    /// in `from_scene`) so its light can go on leaving. So is the mark, and for
    /// the same reason: it is the light's SIZE, and a size that stepped when
    /// the marking voice was pruned snapped a halo still at full brightness.
    glow: [f32; 4],
    /// Which half of the node this instance draws: [`PAINT_WHOLE`] for both,
    /// [`PAINT_INK`] or [`PAINT_CLEARING`] for one. See `Instance::paint` in
    /// lattice.wgsl for what the split buys and why it comes out the same
    /// picture.
    paint: f32,
}

/// What [`GpuInstance::paint`] can say, mirroring lattice.wgsl's constants of
/// the same names. Every sheet but the home one draws WHOLE; a home node is
/// split in two so the resting markers can be drawn between its halves.
///
/// The clearing half rides a buffer of its own rather than extra entries in the
/// instance list, and that is what keeps the split to the scene pass: the glow,
/// the ink strip and the cull all walk the instance buffer whole, and a node
/// appearing twice in it would light twice and write its strip row twice. None
/// of them read this field.
const PAINT_WHOLE: f32 = 0.0;
const PAINT_INK: f32 = 1.0;
const PAINT_CLEARING: f32 = 2.0;

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
            7 => Float32x4, 8 => Float32x4, 10 => Float32, 11 => Float32,
            12 => Float32x4, 13 => Float32
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

/// One marker-pipeline instance: the marker standing at one home-sheet
/// lattice position.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuPlus {
    /// xyz: the position's world center, w: the length of one ARM, crossing to
    /// tip — the quad reaches `PLUS_QUAD_MARGIN` past it, for the soft band to
    /// stand in. Per instance rather than in a uniform because it is a WORLD
    /// length; the two proportions measured against it, the arm's thickness and
    /// where its ends taper, are the same for the whole field and ride in
    /// `misc5.x` and `misc5.y`.
    pos_radius: [f32; 4],
    /// rgb: the marker's own ink, a: its opacity. Both come off one resolve of
    /// `ViewConfig::marker_ink`, so a marker at rest is that grey exactly; the
    /// alpha is under one otherwise, a name claiming the position over it
    /// (`derive_pluses`).
    ///
    /// The alpha is the whole marker's and not only its ink's — the pool it
    /// gives off and the standoff its cross writes into the light are the same
    /// number (`PlusInstance::strength`), so a position handing itself over to
    /// a name hands all three over together.
    color: [f32; 4],
}

impl GpuPlus {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuPlus>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
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
    /// instances, markers, labels and both sets of uniforms, and — on the rare
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
    /// The home sheet's knockouts, one per node that clears, in the order
    /// their nodes are drawn (see [`PAINT_CLEARING`] and [`Draw::Clearing`]).
    clearings: Vec<GpuInstance>,
    /// Every label's glyphs, in the order the pass draws them.
    glyphs: Vec<GlyphInstance>,
    /// One caster per name — its ink's box and its level — in the order the
    /// names are drawn (see [`Draw::Label`]). What `prepare` packs the shadow
    /// atlas from.
    casters: Vec<shadow::Caster>,
    /// The scene pass's whole order, back to front — see [`Draw`].
    draws: Vec<Draw>,
    /// A node's radius on this pane in points, which is the unit the standoff
    /// its names cast is dialled in — see [`LatticeLabels::node_points`].
    node_points: f32,
    /// The two sheets, on the frames either has moved.
    atlas: Option<FontAtlas>,
    marks: Option<FontAtlas>,
    /// Which way these names travel, for the glyph shader's filter.
    slide: SlideAxis,
    pluses: Vec<GpuPlus>,
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

/// One draw the scene pass makes. The pass is a walk over a list of these,
/// back to front, and every index in one addresses the pane's own buffers.
///
/// The order is MATERIALISED rather than reconstructed. Everything the pass
/// draws — the nodes, the home sheet's knockouts, the markers and the names —
/// is depth-sorted once, in `LatticeCallback::from_scene`, and the sequence
/// falls out of that one walk. What that buys is that there is no second
/// expression of the order to keep in step with the first: a draw goes where
/// the walk put it, so nothing has to be told which side of anything else it
/// belongs on.
///
/// The alternative is what this replaces, and it is worth naming because it
/// looks cheaper. Carrying an INDEX into the node run per marker, knockout and
/// name — "how many nodes go in front of me" — costs one tiebreak per
/// collision, and the collisions are unavoidable: a node that paints nothing
/// ships no instance, so it moves no index, and the draws either side of it
/// land on the same number while belonging on opposite sides.
///
/// Depth is what the reader is being shown, so depth is what the order
/// follows. See `from_scene` for the sort and for the spacing inside one node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Draw {
    /// A run of node instances, as a range into `instances`.
    Nodes(u32, u32),
    /// One home node's knockout, by index into `clearings`.
    Clearing(u32),
    /// A run of markers, as a range into `pluses`.
    Pluses(u32, u32),
    /// One name's glyphs, as a range into `glyphs`, plus the index into
    /// `casters` of the name itself — the box its shadow is drawn over.
    ///
    /// Per NAME, and never merged with the name beside it, because a name is
    /// TWO draws: its shadow over its box, then its glyphs. The shadow has to
    /// land on everything already in the frame — the neighbouring name's ink
    /// included, wherever it reaches it — and the ink has to land after its
    /// own shadow; two names as one draw would put the second's shadow under
    /// the first's ink. A name lands at its own node's place in the
    /// back-to-front order, so two names on different nodes overlapping is the
    /// nearer one sitting on, and shadowing, the other.
    Label(u32, u32, u32),
}

/// Add node instance `at` to the run this list ends with, or start a new one.
///
/// A run is exactly a stretch of instances the walk did not interrupt: the
/// buffers are filled in draw order, so "the last draw ends where this one
/// starts" is the whole test.
fn push_node(draws: &mut Vec<Draw>, at: u32) {
    match draws.last_mut() {
        Some(Draw::Nodes(_, end)) if *end == at => *end = at + 1,
        _ => draws.push(Draw::Nodes(at, at + 1)),
    }
}

/// The same for one marker — see [`push_node`].
fn push_plus(draws: &mut Vec<Draw>, at: u32) {
    match draws.last_mut() {
        Some(Draw::Pluses(_, end)) if *end == at => *end = at + 1,
        _ => draws.push(Draw::Pluses(at, at + 1)),
    }
}

/// The markers no node stands at, drawn where the whole field used to be:
/// over the sheets behind the home one, under the home sheet itself.
///
/// See `from_scene` for what makes a marker loose and why it can only be one
/// the caller built by hand.
fn push_loose(draws: &mut Vec<Draw>, pluses: &mut Vec<GpuPlus>, loose: &[GpuPlus]) {
    for &plus in loose {
        push_plus(draws, pluses.len() as u32);
        pluses.push(plus);
    }
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
        let render_scale = scene.render_scale.clamp(RENDER_SCALE_RANGE.0, RENDER_SCALE_RANGE.1);
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
        // is not a cosmetic problem: it puts the markers in the wrong place in
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

        let to_gpu = |n: &harmonigraph_scene::NodeInstance| GpuInstance {
            world_pos: n.world_pos.to_array(),
            color: n.color.to_array(),
            params: [n.activation, n.melody_level, n.bass_level],
            octaves: pack_octaves(&n.octaves),
            cents: n.cents,
            marks: [n.melody_slots, n.bass_slots],
            melody_color: n.melody_color.to_array(),
            bass_color: n.bass_color.to_array(),
            scale: n.scale,
            ring: n.audio_ring,
            glow: [n.glow.level, n.glow.row as f32, n.glow.mix, n.glow.marked],
            paint: PAINT_WHOLE,
        };

        let split = order.iter().position(|&(plane, _, _)| plane <= 0.0).unwrap_or(order.len());
        // And where it ENDS: the sheets in front are the negative half of the
        // same axis, so the home run is exactly the nodes at a depth of 0. A
        // sheet in FRONT keeps its clearing whole — hiding the markers is the
        // ordinary occlusion every sheet does to the one behind it.
        let home_end = order.iter().position(|&(plane, _, _)| plane < 0.0).unwrap_or(order.len());
        // A node that can paint nothing is not shipped at all. The shader
        // already discards it per fragment, but the billboard is deliberately
        // bigger than the node (QUAD_MARGIN and then some), so the discard is
        // paid a fragment at a time over a quad the disc never reaches — and
        // an unplayed lattice is ENTIRELY such nodes: an idle node draws no
        // light of its own and carries no trail mark, so a still lattice ships
        // its markers and nothing else.
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
        //
        // A node's own LIGHT is the one thing here that is not a layer, and it
        // is why the cull is not simply "does this node draw ink": the light is
        // carried on a clock of its own (`panes::glow_fade`), so it outlives
        // every layer that lit it, and a node dropped the frame its last layer
        // went silent would take its whole halo off in one. It ships until the
        // light is over — exactly 0, again rather than nearly, so a shipped
        // instance is always one with something to draw.
        let ringing = scene.spectral.ring_draws();
        let lights = scene.glow_reach > 0.0 && scene.glow_strength > 0.0;
        let paints = |g: &GpuInstance| {
            (ringing && g.ring > 0.0)
                || (lights && g.glow[0] > 0.0)
                || g.params[0] > 0.0
                || g.params[1] > 0.0
                || g.params[2] > 0.0
                || (g.octaves[0] | g.octaves[1] | g.octaves[2]) != 0
        };
        // Which marker stands at each node's position, by index into
        // `scene.pluses`. The crosses are derived off the home nodes
        // (`derive_pluses`), so in the plugin every one of them has a node here
        // and the lattice position is what ties the two lists together. That
        // link is what gives a marker its DEPTH: it stands exactly where its
        // node stands, so the sorted order the nodes are walked in is the
        // order the markers want too.
        //
        // A marker at a position no home node holds is LOOSE, having no node to
        // take a depth from. It keeps the place the whole field used to have —
        // over the sheets behind home, under the home sheet — which is the one
        // place a marker of unknown depth cannot be wrong by more than a sheet.
        // Two home nodes claiming one lattice position is the other way in
        // here, the map keeping the last of them.
        let mut plus_of = vec![u32::MAX; scene.nodes.len()];
        let node_at: std::collections::HashMap<_, usize> = scene
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.on_home)
            .map(|(i, n)| (n.lattice_pos, i))
            .collect();
        for (p, plus) in scene.pluses.iter().enumerate() {
            if let Some(&i) = node_at.get(&plus.lattice_pos) {
                plus_of[i] = p as u32;
            }
        }
        // Read back off `plus_of` rather than written beside it: a position two
        // markers name has only the last of them in the map, and the first is
        // then claimed by nobody and has to stay loose or it is never drawn.
        let mut claimed = vec![false; scene.pluses.len()];
        for &p in &plus_of {
            if p != u32::MAX {
                claimed[p as usize] = true;
            }
        }
        let to_plus = |d: &harmonigraph_scene::PlusInstance| GpuPlus {
            pos_radius: [d.pos.x, d.pos.y, d.pos.z, d.radius],
            color: [d.color.x, d.color.y, d.color.z, d.strength],
        };
        let loose: Vec<GpuPlus> = scene
            .pluses
            .iter()
            .enumerate()
            .filter(|(p, _)| !claimed[*p])
            .map(|(_, d)| to_plus(d))
            .collect();

        // Where each name's glyphs sit in what the caller handed over, per
        // node, so the walk below can put a name at its own node's place in the
        // order rather than working that place out again afterwards. The cursor
        // advances over labels the scene has no node for as much as over the
        // rest: the run lengths are what say which glyphs are whose, and a
        // label naming a node that is not in the scene is dropped rather than
        // drawn somewhere arbitrary — the caller and the scene disagreeing
        // about how many nodes there are is a bug in the caller.
        let mut glyphs_of = vec![(0u32, 0u32); scene.nodes.len()];
        let mut taken = 0u32;
        for label in &labels.labels {
            let start = taken;
            taken = (taken + label.glyphs).min(labels.glyphs.len() as u32);
            if let Some(slot) = glyphs_of.get_mut(label.node as usize) {
                *slot = (start, taken - start);
            }
        }

        // The one walk: every draw the pass makes, emitted in the order it
        // makes them, filling the four buffers as it goes (see [`Draw`]).
        //
        // Inside ONE home node the spacing is its knockout, then the cross
        // standing at its position, then its ink. Three things ride on that,
        // and no coarser order gives all three.
        //
        // A home node clears the home nodes BEHIND IT, which is what the
        // clearing is for wherever two nodes overlap on one sheet — every
        // orbited camera, where the sheet foreshortens under billboards that do
        // not. Batching the knockouts ahead of the ink puts every hole under
        // every ring, so the bar goes quiet on the picture it is most wanted in.
        //
        // A node does NOT clear the cross it stands on. That cross is not
        // behind the node, it is AT it, and a hole punched over it takes the
        // mark of the position while leaving the marker's standoff in the light
        // — a cross-shaped hole with nothing standing in it
        // (`a_node_lit_by_no_key_keeps_the_cross_under_it`). Drawn after that
        // node's own knockout and before its ink, the cross keeps the rule it
        // is meant to keep: it goes when a NAME stands over it and at no other
        // time (`derive_pluses`). A node wearing an audio RING is where the two
        // meet at all: it draws ink with no name over it, so it is the one node
        // that carries a cross and a knockout at once.
        //
        // It still clears every cross BEHIND it, which is the marker field
        // hidden by a node in front of it — the same occlusion the field takes
        // from the sheets in front, and what puts a sounding node in a clean gap
        // in the lattice rather than on top of it. A cross in FRONT of a node
        // covers that node, which is the same rule read the other way and the
        // reason the markers are in this walk at all. Face-on the case does not
        // arise: a node's disc reaches about its own cell, so the only cross
        // under it is its own, and drawing the whole field under the whole sheet
        // is indistinguishable. Tilt the sheet and one disc spans a dozen
        // positions while the billboard does not foreshorten with it, so every
        // cross it covers is one the reader is being told is behind it — and
        // most of them are not.
        let mut instances = Vec::with_capacity(order.len());
        let mut clearings = Vec::new();
        let mut pluses = Vec::with_capacity(scene.pluses.len());
        let mut glyphs = Vec::with_capacity(labels.glyphs.len());
        let mut casters: Vec<shadow::Caster> = Vec::new();
        let mut draws: Vec<Draw> = Vec::with_capacity(order.len());
        let mut loose_drawn = false;
        for (k, &(_, _, i)) in order.iter().enumerate() {
            if k == split {
                push_loose(&mut draws, &mut pluses, &loose);
                loose_drawn = true;
            }
            let home = (split..home_end).contains(&k);
            let instance = to_gpu(&scene.nodes[i]);
            let ships = paints(&instance);
            // The knockout hangs off the Shadow, so a view with it dialled off
            // ships none. The cull is the ink's, since the two are one node — a
            // node that paints nothing clears nothing either, every layer's
            // hole being scaled by the level that paints it (`node_clearing`).
            if ships && home && scene.glow_shadow > 0.0 {
                draws.push(Draw::Clearing(clearings.len() as u32));
                clearings.push(GpuInstance { paint: PAINT_CLEARING, ..instance });
            }
            // The cross, whether or not the node it stands on draws anything:
            // an idle position is exactly where a marker does its work, and the
            // node it belongs to is still what says how far off it is.
            if plus_of[i] != u32::MAX {
                push_plus(&mut draws, pluses.len() as u32);
                pluses.push(to_plus(&scene.pluses[plus_of[i] as usize]));
            }
            if ships {
                push_node(&mut draws, instances.len() as u32);
                // The home sheet's nodes draw twice, so the second of the two
                // takes the ink alone; every other sheet's node is one draw
                // carrying both (`GpuInstance::paint`).
                let paint = if home { PAINT_INK } else { PAINT_WHOLE };
                instances.push(GpuInstance { paint, ..instance });
            }
            // The name, immediately after the node it names — so what covers a
            // name is exactly what covers its node.
            let (start, count) = glyphs_of[i];
            if count > 0 {
                let at = glyphs.len() as u32;
                let run = &labels.glyphs[start as usize..(start + count) as usize];
                glyphs.extend_from_slice(run);
                draws.push(Draw::Label(at, at + count, casters.len() as u32));
                casters.push(shadow::caster_of(run));
            }
        }
        // The home run can be empty and can run to the end of the order, in
        // which case the walk never reached `split`.
        if !loose_drawn {
            push_loose(&mut draws, &mut pluses, &loose);
        }

        LatticeCallback {
            instances,
            clearings,
            glyphs,
            casters,
            draws,
            node_points: labels.node_points,
            atlas: labels.atlas,
            marks: labels.marks,
            slide: labels.slide,
            pluses,
            uniforms: Uniforms {
                view_proj: view_proj.to_cols_array(),
                cam_right: right.extend(0.0).to_array(),
                cam_up: up.extend(0.0).to_array(),
                misc: [0.0, scene.node_radius, scene.glow_meld, 0.0],
                misc2: [
                    scene.darkest_pitch,
                    scene.brightest_pitch,
                    render_scale,
                    bloom_strength(scene.bloom_strength),
                ],
                misc3: [0.0, scene.outer_inner, scene.outer_outer, scene.rings_outer],
                pitch_lut: std::array::from_fn(|k| scene.pitch_lut[k].to_array()),
                misc4: [0.0, scene.mark_inner, 0.0, 0.0],
                misc5: [
                    scene.plus_half_width,
                    scene.plus_taper_start,
                    scene.octave_gap,
                    scene.mark_thickness,
                ],
                misc6: [0.0, 0.0, 0.0, scene.pulse_marks.shader_index() as f32],
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
                // Zeroed whole rather than packed where the glow does not
                // draw — see `Uniforms::misc10`.
                misc10: if lights {
                    [scene.glow_reach, scene.glow_strength, scene.glow_feather, scene.glow_blend]
                } else {
                    [0.0; 4]
                },
                // Packed whatever `lights` says — see `Uniforms::misc11`. The
                // glow-only readers are all inside the glow pass, which
                // `glow_draws` skips entirely, so what a light-less frame does
                // with this row is cut its holes with it and nothing else.
                misc11: [
                    scene.glow_shadow,
                    scene.glow_shadow_soft,
                    scene.glow_shadow_shape,
                    scene.glow_shadow_depth,
                ],
                misc12: if lights {
                    [scene.glow_rows.max(1) as f32, 0.0, 0.0, 0.0]
                } else {
                    [0.0; 4]
                },
                // HALF of it zeroed where the glow does not draw. The wash is a
                // share of the light and goes with it; the unit beside it is
                // what a marker's own HOLE is measured in (`vs_plus`), and a
                // hole is cut with no light in the picture at all — which is
                // `misc11`'s rule above, reaching one field further now that
                // the markers knock out as the nodes do.
                misc13: [if lights { scene.glow_wash } else { 0.0 }, scene.marker_unit, 0.0, 0.0],
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
                spectrum_color: if scene.spectral.ring_draws() {
                    pack_spectrum(&scene.spectral.color_levels)
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

    /// Whether this frame's view asks for a node glow at all — a reach to
    /// spread it over and a strength to draw it at, which `from_scene` has
    /// already reduced to one number. False and nothing is allocated, encoded
    /// or composited: no target, no pass, and a node's clearing reading the
    /// stand-in transparent texture rather than a light. The clearing itself
    /// still cuts — its shape is the Shadow's, which this does not gate (see
    /// [`Uniforms::misc11`]).
    fn glow_draws(&self) -> bool {
        self.uniforms.misc10[0] > 0.0
    }
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct LatticeResources {
    pipeline: wgpu::RenderPipeline,
    plus_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    /// Bloom chain: bright pass, half->quarter downsample, blur x2.
    bright_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    blur_h_pipeline: wgpu::RenderPipeline,
    blur_v_pipeline: wgpu::RenderPipeline,
    /// The node glow's own pass: one draw over the node instance buffer, into
    /// a target of the glow's own (see [`create_glow_pipeline`]).
    glow_pipeline: wgpu::RenderPipeline,
    /// The shadow the resting markers cast into that same target — the other
    /// half of what the glow pass draws (see [`create_glow_pipelines`]).
    plus_glow_pipeline: wgpu::RenderPipeline,
    /// The colour it draws in, settled ahead of it: the ink read round every
    /// node, then blurred (see [`create_ink_strip_pipelines`]).
    ink_strip_pipeline: wgpu::RenderPipeline,
    ink_blur_pipeline: wgpu::RenderPipeline,
    /// ...and the fullscreen draw that lays that target down at the bottom of
    /// the scene pass. Built against the scene pass's own attachments and
    /// depth, which is what makes it usable inside it.
    glow_over_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    /// One sampled texture + the shared sampler: the bloom chain's passes, which
    /// have exactly one to bind. Every reader of the glow target takes
    /// [`glow_layout`](Self::glow_layout) instead, that target being a pair.
    filter_layout: wgpu::BindGroupLayout,
    /// The glow target's TWO textures plus the shared sampler: the light
    /// screen-blended and the same light max-blended, which the Meld bar mixes
    /// between (see [`create_glow_pipeline`]).
    ///
    /// Its own layout rather than [`filter_layout`](Self::filter_layout)
    /// because it holds a second texture, and every other user of that one —
    /// the whole bloom chain — has exactly one to bind. Taken at group 0 by
    /// the composite that lays the light down and at group 1 by the node and
    /// marker pipelines, whose clearings read the same mix.
    glow_layout: wgpu::BindGroupLayout,
    /// A 1x1 transparent texture in [`glow_layout`](Self::glow_layout),
    /// standing in for the glow target at group 1 wherever there is not one.
    ///
    /// Two places there is not, and neither is an error state: the Reach bar at
    /// 0 drops the target entirely (`Offscreen::ensure_glow`), and the
    /// single-attachment `fs_main`/`fs_plus` path the parity test draws through
    /// has no glow pass at all. Transparent light composites to the plain
    /// ground, so `node_paint` needs no branch for either — which is the whole
    /// reason this is a dummy texture rather than a second pipeline variant.
    glow_dummy_bind_group: wgpu::BindGroup,
    /// One texture and NO sampler: the ink strip, which is read texel by texel
    /// (see [`InkStrip`]). Its own layout rather than `filter_layout` because
    /// the two differ in exactly that — a strip is indexed by node and by
    /// angle, and a filtered lookup across its rows would blend one node's
    /// colour into its neighbour's.
    strip_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The node labels: the same glyph shader the rest of the UI's text
    /// draws through (`crate::text`), built for THIS pass — its format and
    /// its depth attachment — so a name takes its place in the scene's own
    /// back-to-front order instead of being laid over the finished picture.
    ///
    /// A name is the resting field's ink, so it takes the light on the field's
    /// terms: the fill is washed by the halo it stands in (`fs_fill_lit`, which
    /// is why this one carries the glow at group 1). What it does to that halo
    /// is its shadow, the draw before it (`shadow_box_pipeline`).
    glyph_fill_pipeline: wgpu::RenderPipeline,
    /// The shadow atlas's three stages (`crate::shadow`): every name's glyphs
    /// into its cell, the blur over the cells along each axis, and the box
    /// each name multiplies the scene by off its finished cell.
    glyph_cell_pipeline: wgpu::RenderPipeline,
    shadow_blur_pipelines: (wgpu::RenderPipeline, wgpu::RenderPipeline),
    shadow_box_pipeline: wgpu::RenderPipeline,
    glyph_layout: wgpu::BindGroupLayout,
    /// How the atlas is read, by the blur and the box alike.
    shadow_layout: wgpu::BindGroupLayout,
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
    /// The home sheet's knockouts, each drawn at its own node
    /// ([`Draw::Clearing`]). A buffer of its own rather than more entries in
    /// the instance list, so the glow, the strip and the cull go on seeing
    /// each node exactly once.
    clearing_buffer: wgpu::Buffer,
    clearing_capacity: usize,
    plus_buffer: wgpu::Buffer,
    plus_capacity: usize,
    plus_count: u32,
    glyph_buffer: wgpu::Buffer,
    glyph_capacity: usize,
    glyph_count: u32,
    /// One box per name — its cell of the shadow atlas and the quad its shadow
    /// is drawn over ([`Draw::Label`]) — and the same box again once per GLYPH,
    /// beside the glyph buffer, for the draw that rasterizes each glyph into
    /// its name's cell (`vs_glyph_cell`). The second is what one instanced
    /// draw over the glyphs costs to know which cell each belongs to; it is
    /// kept at the glyph buffer's own capacity.
    box_buffer: wgpu::Buffer,
    box_capacity: usize,
    box_count: u32,
    cell_buffer: wgpu::Buffer,
    /// The scene pass's whole order (see [`Draw`]), held to what actually
    /// reached the buffers above.
    draws: Vec<Draw>,
    /// What the glyph shader is told about this pane: its size in points, the
    /// atlas's, and the terms a name's standoff is cast on.
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
    /// An ink strip rescued from the target this one replaced, for
    /// [`ensure_glow`](Offscreen::ensure_glow) to adopt instead of building a
    /// fresh one.
    ///
    /// A rebuild here is about the pane's PIXELS, and the strip is rows: it
    /// holds every lit node's colour and nothing about it depends on the size
    /// of the pane. Dropping it costs the one thing a node in its release
    /// cannot replace — such a node draws no layer at all, so its halo's
    /// colour is entirely what the strip already held, and a strip rebuilt
    /// from nothing takes the halo with it in a single frame.
    carried_strip: Option<InkStrip>,
    /// The node glow's own target, present only while the view asks for one.
    glow: Option<GlowTarget>,
    /// The names' shadow atlas, present only while a frame has names casting a
    /// shadow (`ensure_shadow`).
    ///
    /// Its own lifetime rather than a member of [`GlowTarget`], because the two
    /// answer to different bars: a name's shadow lands on the ground at a Reach
    /// of 0, where there is no light and no glow target at all.
    shadow: Option<shadow::ShadowTarget>,
    /// Composite: scene color + blurred bloom (quarter A) + uniforms.
    composite_bind_group: wgpu::BindGroup,
    size: [u32; 2],
    screen_size: [u32; 2],
}

/// Where a frame's node light is assembled before any of it reaches the
/// picture: two transparent premultiplied colour textures at the scene's own
/// size — the same light under each of the pass's two blends — plus the bind
/// group its readers take both through.
///
/// A target of its own, rather than the glow drawn straight into the scene
/// pass, because a node has to sample the finished light to paint its own
/// picture (`node_paint`), and a pass cannot sample the attachment it writes.
/// Every node's halo melds here first (`fs_glow`), across every sheet at once,
/// and the scene pass then lays that one layer down at its bottom and reads it
/// again per node.
///
/// Created and dropped as the Reach bar crosses 0, independently of the resize
/// that rebuilds everything around it: the two changes have nothing to do with
/// each other, and a target left allocated at reach 0 is two scene-sized
/// textures held for a feature that is off.
struct GlowTarget {
    view: wgpu::TextureView,
    /// The same light, blended with MAX instead of screen: an overlap here is
    /// exactly as bright as the brighter of the nodes making it. Its own
    /// attachment because a blend is fixed-function and per-target, so the two
    /// answers can only be had by writing both and mixing after — which is
    /// what the Meld bar dials (`node_paint` in lattice.wgsl, and blit.wgsl's
    /// `fs_glow_over`).
    ///
    /// The cost is one more scene-sized texture while the glow is on. What it
    /// buys is both ends of the bar being EXACT rather than approached: at 1
    /// the picture is the screen blend to the texel, at 0 it is the max blend,
    /// and a pixel one node lights alone is identical at every setting because
    /// the two blends agree wherever only one node writes.
    max_view: wgpu::TextureView,
    /// The STANDOFF's own attachment: how much of the light at each pixel the
    /// lattice's rings hold off (see lattice.wgsl's `glow_shade_tex`).
    ///
    /// A third attachment and not a channel of either light above, because it
    /// takes a blend neither of them does — `max` over a coverage, where those
    /// two are a screen and a max over premultiplied colour, and a blend state
    /// is per target. [`GLOW_SHADE_FORMAT`] is what keeps the cost of that to
    /// half of one of them.
    shade_view: wgpu::TextureView,
    /// All three textures + the shared sampler, as
    /// [`LatticeResources::glow_layout`] takes them.
    bind_group: wgpu::BindGroup,
    /// The colour that light is drawn in, settled once per node per frame.
    strip: InkStrip,
}

/// A frame's ink strips: what every node is putting on itself, read round each
/// of them at [`INK_STRIP_N`] angles and blurred there.
///
/// One ROW per instance, in the instance buffer's own order, which is what a
/// node's `strip_row` indexes. Two textures because the blur cannot read the
/// target it writes: `raw` is `fs_ink_strip`'s reading, `blurred` is
/// `fs_ink_blur`'s convolution of it plus, in one extra column, the same
/// average at no concentration — the mean a node's middle eases toward.
///
/// Small: an f16 RGBA texel per angle per node, so a lattice of 400 lit nodes
/// spends about 400 KB on the pair — a fortieth of the pane's own offscreen at
/// that size. That is what the light costs in memory to stop costing a whole
/// reading of the node per lit fragment.
struct InkStrip {
    /// The raw reading, in a PAIR that ping-pongs: the frame writes one and
    /// reads the other, which is what lets a row hold an average of this
    /// frame's ink and the ink that same row already had (`fs_ink_strip`).
    ///
    /// A node's light is carried on a clock of its own, and the COLOUR half of
    /// that is here — the ink is read in WGSL by the same functions that draw
    /// each layer, so there is nowhere else it could be carried without
    /// spelling every layer's colour a second time in Rust. Which is also why
    /// the row a node writes has to be the row it wrote last frame: this is a
    /// texture read back by identity, and the identity is the row
    /// (`harmonigraph_scene::GlowStep::row`).
    raw_views: [wgpu::TextureView; 2],
    /// Each of the two, as a texture to read: `[parity]` is what the blur takes
    /// and `[parity ^ 1]` is last frame's, which the reading pass mixes into.
    raw_bind_groups: [wgpu::BindGroup; 2],
    blurred_view: wgpu::TextureView,
    /// The blurred strip, as the light's own draw reads it. Not a pair: the
    /// blur is a pure function of the raw strip that has just been written, so
    /// there is nothing in it to carry.
    blurred_bind_group: wgpu::BindGroup,
    /// How many rows the set was built for — the row map's capacity on the
    /// frame that built it, which is what [`Offscreen::ensure_glow`] compares.
    rows: u32,
    /// Which of [`raw_views`](Self::raw_views) this frame writes. Flipped once
    /// per frame, in `prepare`.
    parity: usize,
}

/// How many angles a node's ink is read at, and so how wide its strip is.
/// Mirrors `INK_STRIP_N` in lattice.wgsl, which is where the number is argued;
/// `the_shaders_ink_strip_is_as_wide_as_the_texture_it_is_drawn_into` is what
/// keeps the two one.
const INK_STRIP_N: u32 = 64;

/// What the strip is kept in: an f16 colour per angle, which is what the blur
/// hands the light. Half floats rather than the target's own 8-bit format
/// because a strip texel is a normalised colour beside a WEIGHT, and a weight
/// is a layer's level times its width — small numbers, quantized to nothing by
/// a byte.
const INK_STRIP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// What the standoff layer is kept in: ONE half-float channel, being one
/// coverage in 0..=1 and nothing else (see [`GlowTarget::shade_view`]).
///
/// Two bytes a texel against the four either light beside it takes, the target
/// format being a surface one (BGRA in the plugin, RGBA in the offline
/// renderer). Halving it is the point — the glow's target is already the larger
/// part of a pane's offscreen memory and a coverage has no colour to carry.
///
/// Half floats and not a byte because the layer is MULTIPLIED into premultiplied
/// light rather than composited over it: a coverage quantized to 1/255 puts
/// visible steps across a wide soft band, where the light's own falloff has
/// none.
const GLOW_SHADE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// The shared, pane-independent objects an [`Offscreen`] binds against.
struct OffscreenShared<'a> {
    format: wgpu::TextureFormat,
    composite_layout: &'a wgpu::BindGroupLayout,
    filter_layout: &'a wgpu::BindGroupLayout,
    /// Both of the glow target's textures plus the sampler; see
    /// [`LatticeResources::glow_layout`].
    glow_layout: &'a wgpu::BindGroupLayout,
    /// One texture, unfiltered: what both stages of the ink strip are read
    /// through (see [`InkStrip`]).
    strip_layout: &'a wgpu::BindGroupLayout,
    /// The shadow atlas as its readers take it; see
    /// [`LatticeResources::shadow_layout`].
    shadow_layout: &'a wgpu::BindGroupLayout,
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
    fn run(&self, encoder: &mut wgpu::CommandEncoder, pipelines: BloomPipelines<'_>, label: &str) {
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
        carried_strip: Option<InkStrip>,
    ) -> Self {
        let OffscreenShared { format, composite_layout, filter_layout, sampler, .. } = *shared;
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
                wgpu::BindGroupEntry { binding: 3, resource: uniform_buffer.as_entire_binding() },
            ],
        });

        Offscreen {
            bloom,
            glow: None,
            shadow: None,
            carried_strip,
            composite_bind_group,
            color_view,
            nodes_view,
            depth_view: depth.create_view(&Default::default()),
            size,
            screen_size,
        }
    }

    /// Make this pane's glow target exist exactly while `want` says so.
    ///
    /// Separate from [`Offscreen::new`] because it answers a different
    /// question: `new` runs when the pane's PIXELS change, this when the Reach
    /// bar crosses 0. Folding the two would mean either rebuilding the glow on
    /// every resize or keeping it allocated while the feature is off, and this
    /// is the caller's every-frame path.
    ///
    /// A resize rebuilds the light target, which is the pane's own pixels and
    /// has to be rebuilt, and CARRIES the strip across
    /// (`Offscreen::carried_strip`), which is rows and does not. The two are
    /// not interchangeable: dropping the strip is what takes every node's
    /// colour history with it, and a node in its release has no ink of its own
    /// to seed a new one from.
    ///
    /// `rows` is the row map's own capacity (`Scene::glow_rows`), which grows
    /// and never shrinks within a session — rebuilding the strip is what takes
    /// every node's colour history with it, and the whole set is a few hundred
    /// KB at the sizes a lattice reaches. The light target beside it is
    /// untouched by any of that — it is the pane's own pixels — which is why
    /// only the strip is rebuilt here.
    fn ensure_glow(
        &mut self,
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        want: bool,
        rows: u32,
    ) {
        match (want, self.glow.is_some()) {
            (true, false) => {
                let mut target = GlowTarget::new(device, shared, self.size, rows);
                // A strip rescued from the target this one replaced, where
                // there is one: see `carried_strip`. Only when it is the right
                // height — a strip of the wrong `rows` is rebuilt below anyway,
                // and adopting it first would only move the same work.
                if let Some(strip) = self.carried_strip.take().filter(|s| s.rows == rows) {
                    target.strip = strip;
                }
                self.glow = Some(target);
            }
            (false, true) => self.glow = None,
            _ => {}
        }
        // Nothing carried survives past the frame that could adopt it: holding
        // it longer would hand a stale set of colours to a glow switched back
        // on much later.
        self.carried_strip = None;
        if let Some(glow) = self.glow.as_mut().filter(|g| g.strip.rows != rows) {
            glow.strip = InkStrip::new(device, shared.strip_layout, rows);
        }
    }

    /// Make this pane's shadow atlas hold `want` texels — none at all where
    /// `want` is `None` — the bargain [`ensure_glow`](Offscreen::ensure_glow)
    /// strikes, on the names' shadows instead of the Reach.
    ///
    /// Grown to demand and never shrunk while it is wanted: a layout is a pure
    /// function of its frame (`shadow::pack`), so the texture's size decides
    /// nothing drawn, and a smaller one rebuilt on every frame a name leaves
    /// would be an allocation per frame of a fade. Nothing is carried across a
    /// rebuild the way a glow's ink strip is: the atlas is rewritten from the
    /// ink up every frame it is used.
    fn ensure_shadow(
        &mut self,
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        want: Option<[u32; 2]>,
    ) {
        match want {
            Some(size) => {
                if self.shadow.as_ref().is_none_or(|s| !s.holds(size)) {
                    let held = self.shadow.as_ref().map_or([0, 0], |s| s.size);
                    self.shadow = Some(shadow::ShadowTarget::new(
                        device,
                        shared.shadow_layout,
                        shared.sampler,
                        [size[0].max(held[0]), size[1].max(held[1])],
                    ));
                }
            }
            None => self.shadow = None,
        }
    }
}

impl GlowTarget {
    /// `size` is the SCENE's own pixel size, so the light is drawn at exactly
    /// the resolution the node bodies are and the composite is a texel-aligned
    /// blit. That is what lets `node_paint` read it back with a `textureLoad`
    /// at its own fragment's coordinate: a target at any fraction of the scene
    /// would have to be sampled, and a filtered read of the light under a
    /// node's clearing is a blur nobody asked for.
    fn new(device: &wgpu::Device, shared: &OffscreenShared<'_>, size: [u32; 2], rows: u32) -> Self {
        let OffscreenShared { format, glow_layout, strip_layout, sampler, .. } = *shared;
        // Every attachment at the same SIZE, the standoff included: it is read
        // at the same coordinate as the light it scales, so a target at any
        // other size would be a filtered read of a band with a hard edge in it.
        // The two lights share a FORMAT besides, being one light written under
        // two blends with the mix between them per-texel; the standoff carries
        // one coverage and takes its own (see [`GLOW_SHADE_FORMAT`]).
        let attachment = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
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
        let view = attachment("lattice_glow", format);
        let max_view = attachment("lattice_glow_max", format);
        let shade_view = attachment("lattice_glow_shade", GLOW_SHADE_FORMAT);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_glow_bind_group"),
            layout: glow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&max_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&shade_view),
                },
            ],
        });
        GlowTarget {
            view,
            max_view,
            shade_view,
            bind_group,
            strip: InkStrip::new(device, strip_layout, rows),
        }
    }
}

impl InkStrip {
    /// The set, for a strip `rows` tall. `rows` is floored at one: a texture of
    /// zero height is not a texture, and a spare row nothing draws into is
    /// simply never sampled — the light's own draw is over the instances, each
    /// of which reads the row it just wrote.
    ///
    /// A strip built here holds NOTHING, which is why the frame that builds one
    /// seeds rather than mixing: the clock that hands out rows asks for a
    /// height and knows when its answer changed (`panes::glow_fade` in
    /// harmonigraph-ui), and says so by handing every node a mix of 1.
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, rows: u32) -> Self {
        let tex = |label: &str, width: u32| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width, height: rows.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: INK_STRIP_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        // One column wider than the reading it blurs: the extra one holds the
        // row's MEAN, which is the same convolution at no concentration and so
        // falls out of the same loop (`fs_ink_blur`).
        let raw_views = [
            tex("lattice_ink_strip_raw_a", INK_STRIP_N),
            tex("lattice_ink_strip_raw_b", INK_STRIP_N),
        ];
        let blurred_view = tex("lattice_ink_strip", INK_STRIP_N + 1);
        let bind = |label: &str, view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                }],
            })
        };
        InkStrip {
            raw_bind_groups: [
                bind("lattice_ink_strip_raw_a_bind_group", &raw_views[0]),
                bind("lattice_ink_strip_raw_b_bind_group", &raw_views[1]),
            ],
            blurred_bind_group: bind("lattice_ink_strip_bind_group", &blurred_view),
            raw_views,
            blurred_view,
            rows,
            parity: 0,
        }
    }

    /// The raw strip this frame writes...
    fn writing(&self) -> &wgpu::TextureView {
        &self.raw_views[self.parity]
    }

    /// ...the one it wrote LAST frame, which is what a row's ink is carried
    /// from...
    fn carried(&self) -> &wgpu::BindGroup {
        &self.raw_bind_groups[self.parity ^ 1]
    }

    /// ...and the one it has just written, which the blur reads.
    fn written(&self) -> &wgpu::BindGroup {
        &self.raw_bind_groups[self.parity]
    }
}

/// The two bind group layouts a scene pipeline draws through: the pane's
/// uniforms at group 0, and the finished light at group 1 — the target a node
/// samples to paint the light over the ground rather than the ground bare, and
/// to wash its own ink with that same field (`node_paint`).
///
/// Both the node and the marker pipeline take the pair though only the node's
/// shader touches the light: they are one pass over one pane, so one layout is
/// what lets the light be bound once for both, and a layout naming a binding
/// its shader does not touch costs nothing. Whether there IS a light to bind
/// is the caller's business — see `LatticeResources::glow_dummy_bind_group`.
#[derive(Clone, Copy)]
struct SceneLayouts<'a> {
    uniforms: &'a wgpu::BindGroupLayout,
    glow: &'a wgpu::BindGroupLayout,
}

/// Build one of the scene pipelines from WGSL source (startup uses the
/// baked-in source; hot-reload rebuilds from disk). Node and marker pipelines
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
    layouts: SceneLayouts<'_>,
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
        bind_group_layouts: &[Some(layouts.uniforms), Some(layouts.glow)],
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
        // can tell the node and marker passes apart.
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
    layouts: SceneLayouts<'_>,
    offscreen: bool,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let (node, plus) =
        if offscreen { ("fs_main_scene", "fs_plus_scene") } else { ("fs_main", "fs_plus") };
    (
        create_pipeline(
            device,
            shader_src,
            target_format,
            layouts,
            ("vs_main", node),
            GpuInstance::LAYOUT,
            offscreen,
        ),
        create_pipeline(
            device,
            shader_src,
            target_format,
            layouts,
            ("vs_plus", plus),
            GpuPlus::LAYOUT,
            offscreen,
        ),
    )
}

/// Both pipelines that write the glow's target, from one source: the nodes'
/// light and the shadow the resting markers cast into it.
///
/// They travel together for the reason the scene's pair does — one pass, one set
/// of attachments, so a pipeline built for it is built for both — and for one
/// more of their own: the marker's draw writes the same third attachment a
/// node's rings write their standoff into, on the same Shadow bars.
fn create_glow_pipelines(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    strip_layout: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    (
        create_glow_pipeline(
            device,
            shader_src,
            target_format,
            bind_group_layout,
            strip_layout,
            ("vs_glow", "fs_glow"),
            GpuInstance::LAYOUT,
        ),
        create_glow_pipeline(
            device,
            shader_src,
            target_format,
            bind_group_layout,
            strip_layout,
            ("vs_plus_glow", "fs_plus_glow"),
            GpuPlus::LAYOUT,
        ),
    )
}

/// The glow pass's three attachments and the blends every writer of them melds
/// under: the light screened, the same light max-blended, and the standoff.
///
/// One list, because the terms are what make the field ONE: a node's rings and
/// a marker's cross both write here, and a writer melding on terms of its own
/// would be readable in the picture as the order the draws happened to run in.
///
/// **Attachment 0 is SCREEN**: `src + dst * (1 - src)`, premultiplied on both
/// channels, is what makes two neighbouring nodes' halos MELD: an overlap is
/// brighter than either alone, it is bounded by white however many nodes reach
/// the same pixel, and the operation is commutative, so nothing about the order
/// inside a draw is readable in the picture. Adding instead blows a chord's
/// middle out to white and makes the count of overlapping nodes, rather than
/// any note, the brightest thing on screen.
///
/// **Attachment 1 is MAX**, which is the same guarantee taken all the way: an
/// overlap is exactly as bright as the brighter of the nodes lighting it, so
/// the count of them cannot be read off the picture at all.
///
/// **Attachment 2 is the STANDOFF**, on the same `max` and for a different
/// reason: there it is one of two answers a bar dials between, here it is the
/// only sane way to meld a shadow. The deepest band at a pixel wins, so two
/// emitters' standoffs crossing do not compound into a pit, and the operator is
/// commutative.
///
/// A coverage of 0 is therefore the destination left exactly alone, which is
/// what lets `fs_glow` write one rather than take an early-out it cannot take
/// for the light beside it.
fn glow_targets(target_format: wgpu::TextureFormat) -> [Option<wgpu::ColorTargetState>; 3] {
    let screen = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrc,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    };
    const MAX_COMPONENT: wgpu::BlendComponent = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Max,
    };
    let brightest = wgpu::BlendState { color: MAX_COMPONENT, alpha: MAX_COMPONENT };
    [
        Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(screen),
            write_mask: wgpu::ColorWrites::ALL,
        }),
        Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(brightest),
            write_mask: wgpu::ColorWrites::ALL,
        }),
        Some(wgpu::ColorTargetState {
            format: GLOW_SHADE_FORMAT,
            blend: Some(brightest),
            write_mask: wgpu::ColorWrites::RED,
        }),
    ]
}

/// The node glow's one pipeline: the light, over the node instance buffer and
/// into the glow's own target (see [`GlowTarget`]).
///
/// **TWO attachments, one draw: the same light under both blends.** A blend is
/// fixed-function and per-target, so a dial between two of them can only be had
/// by writing both and mixing after — which is what the Meld bar is
/// (`Scene::glow_meld`), mixed at every reader of this target. The terms are
/// [`glow_targets`]'s, which is where the argument for each of them lives; the
/// max is what the screen alone cannot give at a flat Feather, where a falloff
/// is still near its peak halfway to a neighbour and screening two of those
/// puts more light in the GAP between two nodes than either node has of its
/// own.
///
/// Both are commutative and neither subtracts, so the pair keeps every promise
/// one made: no draw order is readable in either, and a max of premultiplied
/// colour is premultiplied still — each channel is bounded by the alpha of the
/// same fragment, so it is bounded by the largest alpha of any of them.
/// Factors of One on both, min/max ignoring them wherever they are honoured.
///
/// **One draw over every instance**, sheets and all, rather than a sheet at a
/// time: nothing written here is subtractive, so there is nothing for the order
/// to decide. What occludes a node's halo is the scene pass, which draws every
/// node over the finished light — its SHAPE, at least: what the node's own ink
/// then takes of the light under it is `node_paint`'s to say.
///
/// **Its own vertex entry point** (`vs_glow`), because the glow reaches past
/// what a node paints: the billboard has to hold the whole halo, and growing
/// `vs_main`'s quad to match would spend a ring of discarded fragments per node
/// on every frame for a margin no other layer reads.
///
/// **No depth.** The pass this draws into carries none: it is the glow's own,
/// ahead of the scene's, and a screen blend has no order to defend.
///
/// **Two pipelines, one target state.** The nodes' light and the markers'
/// standoff differ only in what they are drawn over: one instance buffer of
/// nodes, one of markers. `entries` and `buffers` are the whole of that
/// difference.
fn create_glow_pipeline(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    strip_layout: &wgpu::BindGroupLayout,
    entries: (&str, &str),
    buffers: wgpu::VertexBufferLayout<'static>,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("lattice_shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });
    // One layout for both, though only the node's light reads the strip: a
    // pipeline may declare a binding its shader never touches, and a second
    // layout differing in nothing else is a second thing to keep in step.
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_glow_pipeline_layout"),
        bind_group_layouts: &[Some(bind_group_layout), Some(strip_layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entries.1),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(entries.0),
            compilation_options: Default::default(),
            buffers: &[buffers],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(entries.1),
            compilation_options: Default::default(),
            targets: &glow_targets(target_format),
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

/// The two passes that settle what colour every node's light is, ahead of the
/// draws that lay it down: the ink read round each node, then blurred.
///
/// **Why a pass at all.** `ink_at` is a function of the NODE and an angle —
/// no uv, no field, no derivative — so a fragment shader evaluating it was
/// answering the same question once per lit pixel, and a node's halo is a lot
/// of pixels. Here it is answered [`INK_STRIP_N`] times per node per frame
/// whatever the zoom, and the light's own draw reads two texels.
///
/// **Why two.** A blur cannot read the target it is writing. The first pass
/// lays the reading down over the instance buffer, one row per node; the second
/// convolves each row with the Spread bar's lobe.
///
/// **Both are over the INSTANCES**, each drawing the one row its node was
/// handed rather than a quad over the whole strip. A strip is as tall as the
/// row map's capacity and a frame lights whatever share of it it lights, so a
/// full-target quad would blur rows nothing wrote — and, worse for the reading
/// pass, would have no instance to take a row and a mix off.
///
/// **The reading is not stateless**, and it is the one thing in the draw path
/// that is not: a row is an average of this frame's ink and what that row
/// already held, so the pass reads last frame's strip
/// (`InkStrip::carried`). Deterministic all the same, and that is what the
/// offline renderer needs: the mix arrives per instance from the frame's own
/// clock, and a render started afresh builds the strips afresh and seeds them
/// on the first frame.
///
/// **No blending on either.** Each writes the row it draws, and a strip texel
/// is a colour and a weight rather than something to composite.
fn create_ink_strip_pipelines(
    device: &wgpu::Device,
    shader_src: &str,
    bind_group_layout: &wgpu::BindGroupLayout,
    strip_layout: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("lattice_shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });
    let build = |label: &str,
                 entry_points: (&str, &str),
                 layout: &wgpu::PipelineLayout,
                 buffers: &[wgpu::VertexBufferLayout<'_>]| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(entry_points.0),
                compilation_options: Default::default(),
                buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry_points.1),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: INK_STRIP_FORMAT,
                    blend: None,
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
    };
    // One layout for the two: each takes the node instances and one strip — the
    // reading takes the strip it is carrying from, the blur the strip that
    // reading has just left.
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_ink_strip_pipeline_layout"),
        bind_group_layouts: &[Some(bind_group_layout), Some(strip_layout)],
        ..Default::default()
    });
    (
        build("fs_ink_strip", ("vs_ink_strip", "fs_ink_strip"), &layout, &[GpuInstance::LAYOUT]),
        build("fs_ink_blur", ("vs_ink_blur", "fs_ink_blur"), &layout, &[GpuInstance::LAYOUT]),
    )
}

/// The draw that lays a finished glow target down at the bottom of the scene
/// pass, before any node, marker or label.
///
/// Not [`create_post_pipeline`], which builds the single-attachment depthless
/// shape every bloom step wants: this one runs mid-pass, so it has to declare
/// what that pass carries — its depth attachment, and its second colour
/// attachment, the labelless copy the bloom reads. blit.wgsl's `fs_glow_over`
/// writes both; the light is part of what the nodes put on screen, so it blooms
/// with the rest of them.
///
/// Depth read-only, like the glyphs': the light takes its place in the pass by
/// draw ORDER, and nothing should ever be occluded by a fullscreen quad.
fn create_glow_over_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    glow_layout: &wgpu::BindGroupLayout,
    uniform_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit_shader"),
        source: wgpu::ShaderSource::Wgsl(BLIT_SRC.into()),
    });
    // The scene's uniforms at a group of ITS OWN, group 0 being the two
    // textures: this pass mixes them by the Meld bar, and the buffer that dial
    // sits in is the one every other stage of the scene reads (`gu` in
    // blit.wgsl).
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_glow_over_pipeline_layout"),
        bind_group_layouts: &[Some(glow_layout), Some(uniform_layout)],
        ..Default::default()
    });
    let target = wgpu::ColorTargetState {
        format: target_format,
        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fs_glow_over"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_blit"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_glow_over"),
            compilation_options: Default::default(),
            targets: &[Some(target.clone()), Some(target)],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
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
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, target_format: wgpu::TextureFormat) -> Self {
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
        let uniform_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_filter_bind_group_layout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        // The glow target's pair, screened and max-blended, which every reader
        // of the light mixes between (see `create_glow_pipeline`), and at 3 the
        // standoff every reader then scales that mix by. The sampler sits among
        // them at 1 rather than after them: the light is read with
        // `textureLoad` and the slot is the filter layout's shape kept, so the
        // one binding both layouts share means the same thing in both.
        let glow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_glow_bind_group_layout"),
            entries: &[texture_entry(0), sampler_entry(1), texture_entry(2), texture_entry(3)],
        });
        // Ahead of the scene pipelines because they take it at group 1: a
        // node paints the finished light over the ground and washes its own ink
        // with it, which means sampling the glow target the pass has just
        // composited.
        let (pipeline, plus_pipeline) = create_pipelines(
            device,
            SHADER_SRC,
            target_format,
            SceneLayouts { uniforms: &bind_group_layout, glow: &glow_layout },
            true,
        );
        // Unfilterable, because every read of it is a `textureLoad`: a row is a
        // node and a column is an angle, so there is no axis a filter would be
        // interpolating along that the shader does not walk itself.
        let strip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_ink_strip_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let (glow_pipeline, plus_glow_pipeline) = create_glow_pipelines(
            device,
            SHADER_SRC,
            target_format,
            &bind_group_layout,
            &strip_layout,
        );
        let (ink_strip_pipeline, ink_blur_pipeline) =
            create_ink_strip_pipelines(device, SHADER_SRC, &bind_group_layout, &strip_layout);

        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_composite_bind_group_layout"),
            entries: &[texture_entry(0), sampler_entry(1), texture_entry(2), uniform_entry(3)],
        });
        let composite_pipeline = create_post_pipeline(
            device,
            "fs_composite",
            target_format,
            &composite_layout,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let glow_over_pipeline =
            create_glow_over_pipeline(device, target_format, &glow_layout, &bind_group_layout);
        let filter =
            |entry| create_post_pipeline(device, entry, target_format, &filter_layout, None);
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
        // The light at group 1, as every other draw in the scene pass takes it:
        // a name is ink standing in it (`fs_fill_lit`).
        let glyph_fill_pipeline = text::create_text_pipeline(
            device,
            target_format,
            &glyph_layout,
            Some(&glow_layout),
            // The plain glyph quad, grown by the reconstruction filter's margin
            // and nothing else: this draw paints the ink alone, the shadow
            // beside it being the box draw's (`fs_shadow_box`).
            ("vs_glyph", "fs_fill_lit"),
            Some(DEPTH_FORMAT),
            EGUI_BLEND,
        );
        let shadow_layout = shadow::read_layout(device);
        let glyph_cell_pipeline = text::create_glyph_cell_pipeline(device, &glyph_layout);
        let shadow_blur_pipelines = shadow::create_blur_pipelines(device, &shadow_layout);
        let shadow_box_pipeline = text::create_shadow_box_pipeline(
            device,
            &glyph_layout,
            &shadow_layout,
            target_format,
            DEPTH_FORMAT,
        );

        // The stand-in light: one transparent texel. It is the format the real
        // target is in so that one bind group layout serves both, and ONE texel
        // because `node_paint` clamps its read into the texture's bounds — so
        // every fragment on screen reads this same nothing, whatever its
        // coordinate, which is exactly what "no light here" means. The clamp
        // is the shader's and not the backend's: WGSL lets an out-of-bounds
        // `textureLoad` answer (0,0,0,1) as readily as zero, and an alpha of
        // 1 here is every clearing painted black.
        //
        // RENDER_ATTACHMENT alongside the binding though nothing ever draws
        // into it: that usage is what gives wgpu a way to zero-initialize the
        // texture, and a zero it cannot write is a texel of whatever the
        // driver left there smeared under every node's clearing.
        let stand_in = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let glow_dummy = stand_in("lattice_glow_dummy", target_format);
        let glow_shade_dummy = stand_in("lattice_glow_shade_dummy", GLOW_SHADE_FORMAT);
        let glow_dummy_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_glow_dummy_bind_group"),
            layout: &glow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&glow_dummy),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                // The same 1x1 transparent texel for both blends: mixing two
                // lots of nothing is nothing at every setting of the Meld bar,
                // which is why there is still no branch here.
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&glow_dummy),
                },
                // A standoff of nothing, which is the light kept WHOLE — the
                // only answer that pairs with a light of nothing, since a pool
                // dimmed out of a field that has no light in it is a dark ring
                // over the bare ground. What has to hold is that x reads 0, and
                // a zero-initialized texel of any float format does that: the
                // layout entry names a SAMPLE TYPE and not a format, so binding
                // `GLOW_SHADE_FORMAT` here buys a stand-in shaped like the
                // attachment it stands in for and nothing the validator checks.
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&glow_shade_dummy),
                },
            ],
        });

        LatticeResources {
            pipeline,
            plus_pipeline,
            composite_pipeline,
            bright_pipeline,
            downsample_pipeline,
            blur_h_pipeline,
            blur_v_pipeline,
            glow_pipeline,
            plus_glow_pipeline,
            ink_strip_pipeline,
            ink_blur_pipeline,
            glow_over_pipeline,
            bind_group_layout,
            composite_layout,
            filter_layout,
            glow_layout,
            glow_dummy_bind_group,
            strip_layout,
            sampler,
            glyph_cell_pipeline,
            shadow_blur_pipelines,
            shadow_box_pipeline,
            shadow_layout,
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
    /// size (pane resizes and render-scale changes recreate it), carrying a
    /// glow target exactly while `glow` asks for one and an ink strip of
    /// exactly `rows`.
    /// `screen_size` is the pane's native (unscaled) pixel size, which
    /// sizes the bloom chain.
    fn pane_buffers(
        &mut self,
        device: &wgpu::Device,
        pane_id: u64,
        offscreen_size: Option<[u32; 2]>,
        screen_size: [u32; 2],
        wants: PaneTargets,
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
            glow_layout: &self.glow_layout,
            strip_layout: &self.strip_layout,
            shadow_layout: &self.shadow_layout,
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
                clearing_buffer: create_vertex_buffer::<GpuInstance>(
                    device,
                    "lattice_clearings",
                    INITIAL_INSTANCE_CAPACITY,
                ),
                clearing_capacity: INITIAL_INSTANCE_CAPACITY,
                plus_buffer: create_vertex_buffer::<GpuPlus>(
                    device,
                    "lattice_pluses",
                    INITIAL_PLUS_CAPACITY,
                ),
                plus_capacity: INITIAL_PLUS_CAPACITY,
                plus_count: 0,
                glyph_buffer: create_vertex_buffer::<GlyphInstance>(
                    device,
                    "lattice_glyphs",
                    INITIAL_GLYPH_CAPACITY,
                ),
                glyph_capacity: INITIAL_GLYPH_CAPACITY,
                glyph_count: 0,
                box_buffer: create_vertex_buffer::<shadow::ShadowBox>(
                    device,
                    "lattice_shadow_boxes",
                    INITIAL_BOX_CAPACITY,
                ),
                box_capacity: INITIAL_BOX_CAPACITY,
                box_count: 0,
                cell_buffer: create_vertex_buffer::<shadow::ShadowBox>(
                    device,
                    "lattice_shadow_cells",
                    INITIAL_GLYPH_CAPACITY,
                ),
                draws: Vec::new(),
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
                // The outgoing target's ink strip comes across. What is being
                // rebuilt is the pane's PIXELS — the strip is rows, and holds
                // every lit node's colour. A node whose note fade has run out
                // draws no layer at all, so its halo's colour is entirely what
                // the strip already held; rebuilt from nothing, every light
                // still running out goes off in one frame, while lights on
                // nodes still holding keys are untouched. That reads as a bug
                // in the release rather than in the resize, and a resize is
                // one drag of a window edge, a dock separator, or a move
                // between displays of different scale.
                let carried = pane.offscreen.take().and_then(|o| o.glow).map(|g| g.strip);
                pane.offscreen = Some(Offscreen::new(
                    device,
                    &shared,
                    &pane.uniform_buffer,
                    size,
                    screen_size,
                    carried,
                ));
            }
            // After the size check rather than inside it: a target rebuilt just
            // above carries no glow, and one kept from last frame may carry the
            // wrong answer. This settles both.
            if let Some(offscreen) = pane.offscreen.as_mut() {
                offscreen.ensure_glow(device, &shared, wants.glow, wants.rows);
                offscreen.ensure_shadow(device, &shared, wants.shadow);
            }
        }
        pane
    }
}

/// Starting element counts for a pane's per-instance and per-marker buffers;
/// both grow by `next_power_of_two` when a frame overflows them.
const INITIAL_INSTANCE_CAPACITY: usize = 256;
const INITIAL_PLUS_CAPACITY: usize = 64;
/// And for its labels. Only sounding, hovered and remembered nodes are named,
/// so a lattice's glyph count is a fraction of a text pane's.
const INITIAL_GLYPH_CAPACITY: usize = 512;

/// And for the names' shadow boxes: one per named node.
const INITIAL_BOX_CAPACITY: usize = 64;

/// Which of a pane's optional targets this frame wants, and how tall the ink
/// strip has to be — the three answers `pane_buffers` acts on that come off the
/// VIEW rather than off the pane's pixels.
///
/// Together rather than three arguments, because they are one question asked
/// once per frame: what does this view need allocated. The pixels beside them
/// (`offscreen_size`, `screen_size`) are a different question and stay separate.
struct PaneTargets {
    /// The node light, which the Reach bar switches (`Offscreen::ensure_glow`).
    glow: bool,
    /// The names' shadow atlas, at the size this frame's cells pack to, or
    /// none where no name casts one (`Offscreen::ensure_shadow`).
    shadow: Option<[u32; 2]>,
    /// The ink strip's height: the row map's own capacity.
    rows: u32,
}

/// A `capacity`-element vertex buffer (VERTEX | COPY_DST) sized for `T`.
/// Used for both the instance and marker buffers, which differ only in label
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
        let resources: &mut LatticeResources =
            callback_resources.get_mut().expect("inserted above when missing");

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
                    let (pipeline, plus_pipeline) = create_pipelines(
                        device,
                        &source,
                        resources.target_format,
                        SceneLayouts {
                            uniforms: &resources.bind_group_layout,
                            glow: &resources.glow_layout,
                        },
                        true,
                    );
                    // The glow off the same source, so an edit to a node's
                    // layers reaches the light around it in the same reload —
                    // they are one shader drawing one node, and reloading half
                    // of it is a halo of the previous build.
                    let (glow_pipeline, plus_glow_pipeline) = create_glow_pipelines(
                        device,
                        &source,
                        resources.target_format,
                        &resources.bind_group_layout,
                        &resources.strip_layout,
                    );
                    // ...and the strip the light is coloured out of, on the
                    // same argument one step further back: an edit to what a
                    // layer paints is an edit to what the halo is made of.
                    let (ink_strip_pipeline, ink_blur_pipeline) = create_ink_strip_pipelines(
                        device,
                        &source,
                        &resources.bind_group_layout,
                        &resources.strip_layout,
                    );
                    resources.pipeline = pipeline;
                    resources.plus_pipeline = plus_pipeline;
                    resources.glow_pipeline = glow_pipeline;
                    resources.plus_glow_pipeline = plus_glow_pipeline;
                    resources.ink_strip_pipeline = ink_strip_pipeline;
                    resources.ink_blur_pipeline = ink_blur_pipeline;
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
        // paints nothing, so a still lattice is exactly a frame of markers and
        // no instances — and keying this on the instances alone would take the
        // markers down with them. So do the LABELS, for the same reason from the
        // other end: a hovered idle node paints nothing and is named, so a
        // lattice can be a frame of one label and nothing else.
        let anything =
            !self.instances.is_empty() || !self.pluses.is_empty() || !self.glyphs.is_empty();
        let offscreen_size = anything.then_some(size);

        let glow = self.glow_draws();
        // The names' shadows: every caster's cell, packed for this frame
        // (`shadow::pack`). None where no name casts one — the Shadow width or
        // depth at the bottom of its bar, or no name in the frame — and that
        // is what takes the atlas and its passes off a resting lattice. σ is
        // in the TARGET's pixels, which is where the cells are drawn and what
        // #496 found the field's reach missing.
        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let sigma =
            shadow::sigma_px(self.uniforms.misc11[0], self.node_points, ppp, self.render_scale);
        let packed = if has_atlas && self.uniforms.misc11[3] > 0.0 {
            shadow::pack(&self.casters, sigma, ppp * self.render_scale, max_dim)
        } else {
            shadow::Packed::default()
        };
        let shadow_wanted = (!packed.boxes.is_empty()).then_some(packed.size);
        let pane = resources.pane_buffers(
            device,
            self.pane_id,
            offscreen_size,
            screen_size,
            PaneTargets {
                glow,
                shadow: shadow_wanted,
                // The strip's height is the row map's CAPACITY, which the
                // light's own clock hands out and which has nothing to do with
                // how many nodes this frame draws (`Scene::glow_rows`).
                rows: self.uniforms.misc12[0] as u32,
            },
        );

        if self.instances.len() > pane.instance_capacity {
            pane.instance_capacity = self.instances.len().next_power_of_two();
            pane.instance_buffer = create_vertex_buffer::<GpuInstance>(
                device,
                "lattice_instances",
                pane.instance_capacity,
            );
        }
        pane.instance_count = self.instances.len() as u32;
        if !self.instances.is_empty() {
            queue.write_buffer(&pane.instance_buffer, 0, bytemuck::cast_slice(&self.instances));
        }

        if self.clearings.len() > pane.clearing_capacity {
            pane.clearing_capacity = self.clearings.len().next_power_of_two();
            pane.clearing_buffer = create_vertex_buffer::<GpuInstance>(
                device,
                "lattice_clearings",
                pane.clearing_capacity,
            );
        }
        if !self.clearings.is_empty() {
            queue.write_buffer(&pane.clearing_buffer, 0, bytemuck::cast_slice(&self.clearings));
        }

        if self.pluses.len() > pane.plus_capacity {
            pane.plus_capacity = self.pluses.len().next_power_of_two();
            pane.plus_buffer =
                create_vertex_buffer::<GpuPlus>(device, "lattice_pluses", pane.plus_capacity);
        }
        pane.plus_count = self.pluses.len() as u32;
        if !self.pluses.is_empty() {
            queue.write_buffer(&pane.plus_buffer, 0, bytemuck::cast_slice(&self.pluses));
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
                pane.cell_buffer = create_vertex_buffer::<shadow::ShadowBox>(
                    device,
                    "lattice_shadow_cells",
                    pane.glyph_capacity,
                );
            }
            pane.glyph_count = self.glyphs.len() as u32;
            if packed.boxes.len() > pane.box_capacity {
                pane.box_capacity = packed.boxes.len().next_power_of_two();
                pane.box_buffer = create_vertex_buffer::<shadow::ShadowBox>(
                    device,
                    "lattice_shadow_boxes",
                    pane.box_capacity,
                );
            }
            pane.box_count = packed.boxes.len() as u32;
            if !packed.boxes.is_empty() {
                queue.write_buffer(&pane.box_buffer, 0, bytemuck::cast_slice(&packed.boxes));
                // Each glyph's own name's box beside it, for the cell draw. The
                // runs are contiguous in draw order, so this is the boxes
                // repeated by their runs' lengths — one per caster, which is
                // what `pack` returns.
                let cells: Vec<shadow::ShadowBox> = self
                    .draws
                    .iter()
                    .filter_map(|draw| match *draw {
                        Draw::Label(a, b, l) => Some((b - a, packed.boxes[l as usize])),
                        _ => None,
                    })
                    .flat_map(|(n, b)| std::iter::repeat_n(b, n as usize))
                    .collect();
                debug_assert_eq!(cells.len(), self.glyphs.len(), "one cell per glyph");
                queue.write_buffer(&pane.cell_buffer, 0, bytemuck::cast_slice(&cells));
            }
            if !self.glyphs.is_empty() {
                queue.write_buffer(&pane.glyph_buffer, 0, bytemuck::cast_slice(&self.glyphs));
                // The glyphs' own points, not the screen's: the rects arrive
                // in this pane's space because the pass they are drawn in is
                // this pane's. `pixels_per_point` stays the DEVICE's — it is
                // what the atlas was rasterized at, and so what turns the
                // rim's radius in points into a texel offset, whatever the
                // render scale does to the target's pixels.
                let atlas_size = pane
                    .offscreen
                    .as_ref()
                    .and_then(|o| o.shadow.as_ref())
                    .map_or([0.0; 2], |s| [s.size[0] as f32, s.size[1] as f32]);
                queue.write_buffer(
                    &pane.glyph_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&text::TextUniforms {
                        screen_points: self.size_points,
                        atlas_size: [sheet_sizes[0], sheet_sizes[1]],
                        mark_atlas_size: [sheet_sizes[2], sheet_sizes[3]],
                        filter_axis: self.slide.unit(),
                        pixels_per_point: screen_descriptor.pixels_per_point.max(f32::EPSILON),
                        // What the names share with the rest of the picture,
                        // read out of the rows they were packed into rather
                        // than off the scene a second time: the Meld a name's
                        // wash mixes the light by, and the Shadow depth its
                        // shadow is spent at. The same numbers the nodes and
                        // markers took, so the bar means one thing. The
                        // Shadow's WIDTH is not here: it is σ, spent on the
                        // CPU where the cells are packed.
                        meld: self.uniforms.misc[2],
                        shadow_depth: self.uniforms.misc11[3],
                        _pad: 0.0,
                        // The atlas the cells are drawn into, which may be
                        // larger than this frame's layout (`ensure_shadow`).
                        shadow_atlas_size: atlas_size,
                        _pad2: [0.0; 2],
                        // A lattice name paints no rim, so there is no ring for
                        // the two passes that draw one to walk. Zero samples is
                        // what says so (`ring`), and it is what keeps the fill's
                        // quad down to the reconstruction filter's own margin.
                        ring0: [0.0; 4],
                        ring1: [0.0; 4],
                    }),
                );
            }
        } else {
            pane.glyph_count = 0;
            pane.box_count = 0;
        }

        // The order, held to what actually reached the buffers: every index in
        // a `Draw` addresses one of four lists and the pass draws off them
        // without a second look, so a draw naming something they do not hold is
        // dropped here rather than trusted there. The names go the same way
        // when there is no atlas — nothing to sample.
        pane.draws.clear();
        pane.draws.extend(self.draws.iter().copied().filter(|draw| match *draw {
            Draw::Nodes(a, b) => a < b && b <= pane.instance_count,
            Draw::Clearing(i) => (i as usize) < self.clearings.len(),
            Draw::Pluses(a, b) => a < b && b <= pane.plus_count,
            Draw::Label(a, b, l) => {
                a < b && b <= pane.glyph_count && (l as usize) < self.casters.len()
            }
        }));

        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
        let write_ms = write_start.elapsed().as_secs_f32() * 1000.0;

        let scene_start = std::time::Instant::now();
        // The scene pass: draw into the pane's offscreen target, on the
        // encoder egui-wgpu executes before its own render pass. paint()
        // then just composites the finished texture.
        // Mutable, for the one thing a pane carries from one frame to the next:
        // which of the ink strip's two raw textures this frame writes (see
        // [`InkStrip`]).
        let pane = resources.panes.get_mut(&self.pane_id).expect("created by pane_buffers above");
        if let Some(glow) = pane.offscreen.as_mut().and_then(|o| o.glow.as_mut()) {
            glow.strip.parity ^= 1;
        }
        let pane = resources.panes.get(&self.pane_id).expect("created by pane_buffers above");
        let draws = pane.instance_count > 0 || pane.plus_count > 0 || pane.glyph_count > 0;
        if let Some(offscreen) = pane.offscreen.as_ref().filter(|_| draws) {
            // Bracket the scene pass and the bloom chain together: what the
            // overlay wants is the cost of drawing THE LATTICE, which is both.
            // Skipped while a readback is still in flight, so the query set is
            // never overwritten mid-cycle.
            let timing =
                self.drives_timer() && resources.timer.as_ref().is_some_and(GpuTimer::arming);
            let opening =
                if timing { resources.timer.as_ref().and_then(GpuTimer::opening) } else { None };

            // The names' shadow atlas, ahead of the scene pass that samples
            // it: every name's glyphs into its cell, then the blur over the
            // cells. Present exactly while a name casts a shadow this frame
            // (`PaneTargets::shadow`), so a resting lattice pays none of it —
            // and skipped along with the boxes whenever the pane carries no
            // glyphs, which is what lets the atlas go unwritten rather than
            // cleared.
            let atlas = offscreen.shadow.as_ref().filter(|_| {
                pane.box_count > 0 && pane.glyph_count > 0 && pane.glyph_bind_group.is_some()
            });
            if let Some(atlas) = atlas {
                let glyphs = pane.glyph_bind_group.as_ref().expect("filtered just above");
                let mut pass = atlas.ink_pass(egui_encoder);
                pass.set_pipeline(&resources.glyph_cell_pipeline);
                pass.set_bind_group(0, glyphs, &[]);
                pass.set_vertex_buffer(0, pane.glyph_buffer.slice(..));
                pass.set_vertex_buffer(1, pane.cell_buffer.slice(..));
                pass.draw(0..4, 0..pane.glyph_count);
                drop(pass);
                let (blur_x, blur_y) = &resources.shadow_blur_pipelines;
                atlas.blur(egui_encoder, (blur_x, blur_y), &pane.box_buffer, pane.box_count);
            }

            // The node glow, into a target of its own and BEFORE the scene
            // pass, which composites it at its bottom and samples it per node.
            //
            // One draw over the whole instance buffer, every sheet at once:
            // both blends the pass writes are commutative and neither
            // subtracts, so there is nothing for a per-sheet walk to decide —
            // what hides a node's halo is the scene pass drawing a nearer node
            // over it.
            //
            // Encoded whenever the target exists, nodes or none: the pass
            // CLEARS it, and a frame that skipped it would composite whatever
            // the last frame left there — light around nodes that are no longer
            // on screen. A lattice can be a frame of markers and labels with
            // every node culled, which is exactly that frame.
            if let Some(glow) = offscreen.glow.as_ref() {
                // What colour that light is, before any of it is laid down: the
                // ink read round every node, then blurred (see [`InkStrip`]).
                // Both are skipped with no instances to read — the light's own
                // draws are too, so nothing samples what they would leave.
                if pane.instance_count > 0 {
                    let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("lattice_ink_strip_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: glow.strip.writing(),
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                // Cleared, though what the pass leaves behind
                                // is not this frame's ink alone: every row a
                                // node holds is written whole, and the rows in
                                // between are ones no node has been handed. It
                                // is what a row that has just been handed BACK
                                // needs — the next node to take it seeds off
                                // its own reading rather than off a stranger's
                                // ink two frames old.
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    // The strip this same pass wrote last frame: what a node's
                    // light is carried FROM (see [`InkStrip`]).
                    pass.set_bind_group(1, glow.strip.carried(), &[]);
                    pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                    pass.set_pipeline(&resources.ink_strip_pipeline);
                    pass.draw(0..4, 0..pane.instance_count);
                    drop(pass);

                    let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("lattice_ink_blur_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &glow.strip.blurred_view,
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
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    pass.set_bind_group(1, glow.strip.written(), &[]);
                    pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                    pass.set_pipeline(&resources.ink_blur_pipeline);
                    pass.draw(0..4, 0..pane.instance_count);
                }

                // The same light under two blends (`create_glow_pipeline`):
                // screened, and taken at its brightest. Then the STANDOFF cut
                // into it, which is a third quantity rather than a third blend
                // of the first. All three clear to transparent, which is the
                // identity for every blend here — a screen over nothing and a
                // max against nothing are both the source, and a coverage of
                // zero is no ring holding any light off.
                let light_attachment = |view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                };
                let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("lattice_glow_pass"),
                    color_attachments: &[
                        light_attachment(&glow.view),
                        light_attachment(&glow.max_view),
                        light_attachment(&glow.shade_view),
                    ],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                // Every node at once, in whatever order the instance buffer
                // holds them. Both blends are commutative and neither
                // subtracts, so this target is one field of light with no depth
                // in it at all — which is what makes it safe to lay under every
                // sheet as a single layer.
                if pane.instance_count > 0 {
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    pass.set_bind_group(1, &glow.strip.blurred_bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                    pass.set_pipeline(&resources.glow_pipeline);
                    pass.draw(0..4, 0..pane.instance_count);
                }
                // The resting markers, into the same three attachments: the
                // standoff each cross holds the light above off by. It draws in
                // the same pass because it is the same field — one light with
                // one shadow cut into it, laid down under the whole lattice,
                // and a marker arriving in a pass of its own would be a second
                // layer for the composite and every clearing to read.
                //
                // Order-free like the node draw beside it, on the same
                // guarantee: the shade blend is `max`, so nothing decides which
                // of a marker and a node reaches a pixel first.
                //
                // Both groups set here rather than left over from the draw
                // above, which is skipped whenever every node is culled — a
                // resting lattice being exactly that frame, and the one this
                // draw exists for.
                if pane.plus_count > 0 {
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    pass.set_bind_group(1, &glow.strip.blurred_bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.plus_buffer.slice(..));
                    pass.set_pipeline(&resources.plus_glow_pipeline);
                    pass.draw(0..4, 0..pane.plus_count);
                }
            }

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

            // The light, FIRST — under every node, marker and label in the
            // pass, which is what makes a node a lamp rather than a hole.
            //
            // The node's own KNOCKOUT is the half of that which is easy to
            // leave out. A node paints the ground filled to its CENTRE, one
            // Shadow out past every layer (`node_clearing`), so light laid
            // down first and then cleared to BARE ground is stamped out
            // exactly where it is most wanted and the feature comes out as a
            // ring of haze round a hole. The node samples this same target
            // instead (`node_paint`), painting the light standing at its own
            // pixel over that ground and washing its own ink with that same
            // field, so a node's middle keeps the light while a
            // nearer node still hides the SHAPE of what is behind it — a
            // covered node's halo is in the field like everyone else's, and
            // being light and nothing else it can only brighten what stands
            // over it.
            //
            // It writes BOTH attachments (`fs_glow_over`), so the bloom's
            // bright pass reads the light exactly as it reads the nodes: it is
            // light the nodes emit, and it blooms with the rest of them.
            if let Some(glow) = offscreen.glow.as_ref() {
                pass.set_pipeline(&resources.glow_over_pipeline);
                pass.set_bind_group(0, &glow.bind_group, &[]);
                pass.set_bind_group(1, &pane.bind_group, &[]);
                pass.draw(0..4, 0..1);
            }

            // That same target at group 1 of every node and marker draw, for
            // the clearings to read back. The dummy where the light does not
            // exist at all, which is the Reach bar at 0 — a transparent read
            // is the plain ground, so nothing branches (see
            // `glow_dummy_bind_group`).
            let light =
                offscreen.glow.as_ref().map_or(&resources.glow_dummy_bind_group, |g| &g.bind_group);

            // The order, as `from_scene` laid it down (see [`Draw`]). One walk
            // forward, back to front, with nothing here deciding what goes
            // where — a name is covered by exactly what covers its node, a
            // knockout falls between its node's two draws, and a cross covers
            // the nodes behind it and no others, because that is where each of
            // them was emitted.
            for draw in &pane.draws {
                match *draw {
                    Draw::Nodes(a, b) => {
                        pass.set_pipeline(&resources.pipeline);
                        pass.set_bind_group(0, &pane.bind_group, &[]);
                        pass.set_bind_group(1, light, &[]);
                        pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                        pass.draw(0..4, a..b);
                    }
                    // The same pipeline and the same bind groups as the nodes:
                    // this IS one of them, drawn for one of the two things it
                    // paints (`PAINT_CLEARING`).
                    Draw::Clearing(i) => {
                        pass.set_pipeline(&resources.pipeline);
                        pass.set_bind_group(0, &pane.bind_group, &[]);
                        pass.set_bind_group(1, light, &[]);
                        pass.set_vertex_buffer(0, pane.clearing_buffer.slice(..));
                        pass.draw(0..4, i..i + 1);
                    }
                    Draw::Pluses(a, b) => {
                        pass.set_pipeline(&resources.plus_pipeline);
                        pass.set_bind_group(0, &pane.bind_group, &[]);
                        pass.set_bind_group(1, light, &[]);
                        pass.set_vertex_buffer(0, pane.plus_buffer.slice(..));
                        pass.draw(0..4, a..b);
                    }
                    // Two draws per name: its shadow over everything already
                    // in the frame under its box, then the glyphs themselves,
                    // washed by the light they stand in. A name paints no rim
                    // — what keeps a halo off it is the shadow, which is a
                    // multiply on the frame and so lands on the light along
                    // with everything else under it (`fs_shadow_box`).
                    //
                    // The SHADOW FIRST, which is what keeps a name's own ink
                    // out of its own shadow: the blend's ink term is not
                    // multiplied, so a name's letters are the one thing in the
                    // frame its shadow never darkens.
                    //
                    // The light at group 1 for the fill, the same bind group
                    // the nodes and markers above it took; the atlas at group
                    // 2 for the box, which reads its own cell and nothing else.
                    Draw::Label(a, b, l) => {
                        let Some(bind_group) = pane.glyph_bind_group.as_ref() else {
                            continue;
                        };
                        pass.set_bind_group(0, bind_group, &[]);
                        pass.set_bind_group(1, light, &[]);
                        if let Some(atlas) = atlas.filter(|_| l < pane.box_count) {
                            pass.set_bind_group(2, &atlas.reads[0], &[]);
                            pass.set_vertex_buffer(0, pane.box_buffer.slice(..));
                            pass.set_pipeline(&resources.shadow_box_pipeline);
                            pass.draw(0..4, l..l + 1);
                        }
                        pass.set_vertex_buffer(0, pane.glyph_buffer.slice(..));
                        pass.set_pipeline(&resources.glyph_fill_pipeline);
                        pass.draw(0..4, a..b);
                    }
                }
            }
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
        // Nothing was rendered into the offscreen target. The markers and the
        // labels count as much as the nodes here — see `prepare`, where the
        // same test decides whether the target exists at all.
        if pane.instance_count == 0 && pane.plus_count == 0 && pane.glyph_count == 0 {
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
