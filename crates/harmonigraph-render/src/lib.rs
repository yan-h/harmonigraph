//! The wgpu lattice renderer, packaged as an egui paint callback.
//!
//! The same code path runs in both shells: the standalone harness (eframe
//! with the wgpu backend) and the plugin editor (egui-baseview with the wgpu
//! backend). A pane that wants to show the lattice allocates a rect and
//! adds [`lattice_paint_callback`] to the painter; pipelines and buffers are
//! created lazily on first paint and cached in egui-wgpu's
//! `CallbackResources`. The plugin retains compiled handles between windows
//! through [`LatticePipelineCache`]; published atlases and pane history stay window-owned.
//!
//! Rendering model: one instanced draw of camera-facing quads (billboards),
//! sorted back-to-front on the CPU, rendered in `prepare()` into a per-pane
//! offscreen color target and composited into the egui pass in
//! `paint()` as one textured quad (blit.wgsl). Owning the pass is what
//! makes the render-scale option (super/sub-sampling) possible, and gives
//! post-processing (bloom etc.) a texture to read. Painter order supplies all
//! occlusion; there is no depth attachment.
//! `offscreen_composite_matches_direct_draw` in the tests pins
//! down that this path matches drawing straight into the egui pass.
//!
//! The node NAMES are drawn in that same pass, each at its own node's place
//! in the order (see [`LatticeLabels`]) — so a nearer node covers the name of
//! the node behind it by ordinary alpha blending, exactly as it covers the
//! sheet behind it. They arrive as glyphs, from the same collector the rest
//! of the UI's text goes through; what differs is which pass they land in,
//! and so that they inherit its render scale. They do NOT reach the bloom:
//! while bloom is on the pass carries a second pair of colour attachments
//! without their ink or coverage, and the bright pass reads their sum.
//!
//! With the `hot-reload` feature (enabled by the standalone harness), the
//! .wgsl files are watched on disk and every pipeline cut from them rebuilds
//! on save — the lattice's and the names', which share common.wgsl. Both
//! modules are validated first, and a broken edit to either logs an error and
//! keeps the pipelines it has instead of crashing or reloading half the
//! picture. Release plugin builds keep `include_str!` only.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use harmonigraph_scene::Scene;

mod ink_history;
mod lattice_frame;
mod lattice_prepare;

pub mod shader_assets;
/// Progress for interactive editor initialization; offline rendering remains synchronous.
pub mod startup;

/// The piano roll's own callback — a different picture with the same
/// problem, solved the same way. It shares this crate's wgpu version, buffer
/// helpers and [`BloomChain`]; the lattice's offscreen target
/// are beside the point for a flat ribbon.
mod roll;
pub use roll::{roll_paint_callback, RollAxes, RollInstance};

/// The spectrogram's heatmap — the pane's other heavy layer, and the one whose
/// picture a CPU compose would build texel by texel. Here the aggregator's slab
/// grid is the texture and the read is per fragment, so a zoom, a resize or a
/// palette change is a uniform.
mod spectrogram;
pub use spectrogram::{
    spectrogram_paint_callback, SpectrogramGrid, SpectrogramHeadless, SpectrogramRead,
    SpectrogramShades, SpectrogramVertex,
};

mod dot_shadow;
/// A halo alone, over marks a pane drew for itself — the third caller of
/// [`BloomChain`], and the one that draws no picture of its own.
mod glow;
mod lattice_node_glow;
pub use dot_shadow::dot_shadow_paint_callback;
pub use glow::{glow_paint_callback, GlowDot};
#[cfg(test)]
use lattice_node_glow::halo_pixels;
use lattice_node_glow::{glow_node_buffer, glow_node_layout, GpuGlowNode};

/// Label text, for the same reason the roll has its own callback: what a
/// label costs is the rim, and the rim was the text drawn again once per
/// stamp.
mod text;
pub use text::{
    text_paint_callback, FontAtlas, GlyphInstance, GlyphSdfAtlas, SlideAxis, GLYPH_SDF_COARSE_PAD,
    GLYPH_SDF_NEAR_BLEND, GLYPH_SDF_NEAR_PAD,
};

/// Generic shadow packing and kernels, shared by every group.
mod shadow;
pub use shadow::{spectral_shadow_reach, SPECTRAL_WIDTH_POINTS};
/// One combined atlas and blur schedule per spectral destination surface.
mod spectral_shadow;
pub use spectral_shadow::spectral_shadow_prepare_callback;

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
/// Bloom reads its own label-free component pair; see [`LatticeBloom`].
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
    /// A CPU font-atlas snapshot for shells that cannot publish egui's current
    /// renderer texture, on the frames that fallback is stale.
    pub atlas: Option<FontAtlas>,
    /// And the drawn marks' own sheet, on the frames it has moved.
    pub marks: Option<FontAtlas>,
    /// The fixed distance sheet every glyph's `sdf_*` rectangles address.
    /// Published with a non-empty batch; unlike the coverage sheets it never
    /// changes after startup.
    pub sdf: Option<GlyphSdfAtlas>,
    /// The axes these names travel along, for the reconstruction filter — see
    /// [`SlideAxis`]. An orbiting camera moves a node name both ways at once,
    /// so the UI hands lattice labels [`SlideAxis::Both`].
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

/// The half of a shader module that lattice.wgsl and text.wgsl both need —
/// the light's texture and the read of it, the wash, the shadow
/// atlas and the transmittance a caster multiplies the frame by, and the
/// arithmetic that maps a caster's box onto its cell. The file itself states
/// what is allowed in there.
pub(crate) const COMMON_SRC: &str = include_str!("shaders/common.wgsl");

/// One module's WGSL: `common.wgsl` and the module's own source, concatenated.
///
/// WGSL has no include and naga takes a string, so this IS the linkage between
/// the two files, and every `create_shader_module` for a module that names
/// anything in common.wgsl is handed the result. Common goes first, so a module
/// can call into it and it can call into no module.
///
/// A module that names nothing in it is compiled as it stands. Handing one the
/// common half anyway would redeclare `glow_tex` at blit.wgsl's own slot for
/// it, which is a compile error rather than a tidy-up.
pub(crate) fn module_source(common: &str, module: &str) -> String {
    format!("{common}\n{module}")
}

/// [`module_source`] against the common half baked into this build, which is
/// every caller but the hot-reload path's.
pub(crate) fn with_common(module: &str) -> String {
    module_source(COMMON_SRC, module)
}

/// How many lines of a concatenated module belong to the common half.
///
/// The offset every naga diagnostic carries: it counts lines in the string it
/// was handed, and the module's own file starts after this many.
///
/// Counted off the PREFIX [`module_source`] builds rather than off the common
/// half alone, which makes it right by construction instead of by three cases:
/// the separator adds a line where common already ends in a newline and ends
/// common's last line where it does not.
#[cfg(any(test, feature = "hot-reload"))]
fn common_lines(common: &str) -> usize {
    module_source(common, "").lines().count()
}

/// The whole text module as this build should compile it: baked, or — under
/// hot-reload, once a reload has been committed — what the watcher read off
/// disk.
///
/// EVERY glyph pipeline is built from this, the lattice's three included: a
/// name's fill, the cell its shadow is blurred from and the box that shadow is
/// spent over are one shader drawing one name.
pub(crate) fn text_source() -> String {
    #[cfg(feature = "hot-reload")]
    {
        reload::text_source()
    }
    #[cfg(not(feature = "hot-reload"))]
    {
        with_common(text::TEXT_SRC)
    }
}

/// The roll and spiral-dot modules joined to the common half currently in
/// force. Their own files remain baked; a common-shader reload still rebuilds
/// them so every group uses one generation of the two kernels.
pub(crate) fn roll_source() -> String {
    #[cfg(feature = "hot-reload")]
    {
        module_source(&reload::common_source(), roll::ROLL_SRC)
    }
    #[cfg(not(feature = "hot-reload"))]
    {
        with_common(roll::ROLL_SRC)
    }
}

pub(crate) fn dot_shadow_source() -> String {
    #[cfg(feature = "hot-reload")]
    {
        module_source(&reload::common_source(), dot_shadow::SRC)
    }
    #[cfg(not(feature = "hot-reload"))]
    {
        with_common(dot_shadow::SRC)
    }
}

/// What a committed reload leaves for the pipelines it cannot reach itself.
///
/// The reload runs inside the lattice callback, which holds one entry of
/// `CallbackResources` and cannot reach the text callback's entry beside it —
/// yet both are built from modules that share common.wgsl, so one edit is due
/// to both. What crosses is the text module and a COUNT of reloads:
/// `TextResources` compares that count exactly as it compares `target_format`,
/// the two saying the same thing, that the pipelines in hand were built for
/// something else.
#[cfg(any(test, feature = "hot-reload"))]
mod reload {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::RwLock;

    static TEXT: RwLock<Option<String>> = RwLock::new(None);
    static COMMON: RwLock<Option<String>> = RwLock::new(None);
    static GENERATION: AtomicU64 = AtomicU64::new(0);

    /// Held by every test that publishes or asks whether a build is current.
    /// Both are process-wide, so two such tests interleaving would each read
    /// the other's reload — and the failure would land on whichever ran second,
    /// which is not the one that is wrong.
    #[cfg(test)]
    pub(crate) static PUBLISH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The lock, poison ignored: a test that panicked while holding it has
    /// already failed, and taking the rest down with it hides which one.
    #[cfg(test)]
    pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        PUBLISH_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// How many reloads have been committed; 0 for a session that has had none.
    pub(super) fn generation() -> u64 {
        GENERATION.load(Ordering::Acquire)
    }

    /// Hand over a reloaded text module.
    ///
    /// Stored BEFORE the count is raised, and the pair ordered Release/Acquire,
    /// so a reader that sees the new count reads the new source with it. The
    /// other order hands out a raised count over the previous source, which is
    /// a rebuild that produces the build it was called to replace and then
    /// reports itself done.
    pub(super) fn publish(text: String, common: String) {
        *TEXT.write().expect("no reader panics while holding this") = Some(text);
        *COMMON.write().expect("no reader panics while holding this") = Some(common);
        GENERATION.fetch_add(1, Ordering::Release);
    }

    /// The text module to compile now: the last one published, or the baked
    /// halves while no reload has been committed.
    pub(super) fn text_source() -> String {
        TEXT.read()
            .expect("no writer panics while holding this")
            .clone()
            .unwrap_or_else(|| crate::with_common(crate::text::TEXT_SRC))
    }

    #[cfg(feature = "hot-reload")]
    pub(super) fn common_source() -> String {
        COMMON
            .read()
            .expect("no writer panics while holding this")
            .clone()
            .unwrap_or_else(|| crate::COMMON_SRC.to_owned())
    }
}

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

/// The lattice's colour between its own passes: the node light, all scene
/// attachments, and every stage of the bloom chain.
///
/// The host surface is normally an 8-bit `Unorm` texture, which is the right
/// final format and the wrong working one for a slow gradient. Reusing it here
/// rounds a halo once when its nodes meld, again when it enters the scene, and
/// once at every bloom hop. As the light fades, those fixed byte boundaries
/// move across its radial falloff as visible rings. Half floats keep the field
/// continuous until `fs_composite` dithers the one unavoidable 8-bit write in
/// [`LatticeCallback::paint`].
const LATTICE_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Clamp on the render-scale view setting, over whatever the UI offers.
const RENDER_SCALE_RANGE: (f32, f32) = (0.25, 4.0);

/// Entry points a (re)loaded shader must provide. The `_scene` pair is the
/// four-attachment form with bloom; `_split` writes the visible pair; the bare pair is
/// the single-attachment one the parity test's reference path uses; the
/// `glow_gather` pair is the light's own pass — one quad over the whole target
/// and the fold that walks every lit node at each of its pixels; the `ink` four
/// are the strip the nodes' light is coloured out of, read and then blurred
/// ahead of it (see [`InkStrip`]); and the `cell` four rasterize a node's ink
/// and the Gaussian's one marker cross into the shadow atlas (`shadow.rs`).
#[cfg(any(test, feature = "hot-reload"))]
const LATTICE_ENTRY_POINTS: &[&str] = &[
    "vs_main",
    "fs_main",
    "fs_main_scene",
    "fs_main_split",
    "vs_plus",
    "fs_plus",
    "fs_plus_scene",
    "fs_plus_split",
    "vs_glow_gather",
    "fs_glow_gather",
    "vs_ink_strip",
    "fs_ink_strip",
    "vs_ink_blur",
    "fs_ink_blur",
    "vs_node_cell",
    "fs_node_cell",
    "vs_plus_cell",
    "fs_plus_cell",
];

/// The two modules a reload rebuilds, each already carrying the common half
/// it was read beside.
#[cfg(any(test, feature = "hot-reload"))]
#[cfg_attr(all(test, not(feature = "hot-reload")), allow(dead_code))]
struct ReloadedShaders {
    common: String,
    lattice: String,
    text: String,
    /// Lines the common half takes in both modules above — the seam a naga
    /// diagnostic's line number has to be read against.
    ///
    /// Carried rather than recomputed from `COMMON_SRC`, because these two
    /// were built against the common half on DISK: once an edit there has
    /// added or removed a line, the baked seam is wrong by that many for
    /// every message from then on, which is the failure the banner exists to
    /// prevent rather than to commit.
    seam: usize,
}

/// Watches the three files those two modules are made of, on disk (dev builds
/// only). The first sighting only records a baseline mtime; edits after launch
/// trigger reloads.
///
/// ALL THREE, and all three are re-read from disk on a reload. common.wgsl is
/// the half BOTH modules are compiled against, so an edit to the wash or to
/// `shadow_transmittance` is an edit to what a node is drawn with and to what a
/// name is drawn with at once. Leaving any half where it was reloads a picture
/// against arithmetic the files on disk no longer hold, and says nothing on
/// screen about which half it kept.
#[cfg(any(test, feature = "hot-reload"))]
struct ShaderWatcher {
    lattice: std::path::PathBuf,
    text: std::path::PathBuf,
    common: std::path::PathBuf,
    /// The NEWEST of the three files' mtimes: one stamp for the set, so an edit
    /// to any of them is one reload of everything they make together.
    mtime: Option<std::time::SystemTime>,
    next_check: std::time::Instant,
}

#[cfg(any(test, feature = "hot-reload"))]
impl ShaderWatcher {
    #[cfg(feature = "hot-reload")]
    fn new() -> Self {
        Self::watching(
            std::path::PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/lattice.wgsl"
            )),
            std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders/text.wgsl")),
            std::path::PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/common.wgsl"
            )),
        )
    }

    /// The three paths spelled out, which is what makes the reload testable:
    /// every path [`new`](Self::new) takes is a source file of this crate, and
    /// what the watcher has to be asked is what it does when one of them is
    /// EDITED.
    fn watching(
        lattice: std::path::PathBuf,
        text: std::path::PathBuf,
        common: std::path::PathBuf,
    ) -> Self {
        ShaderWatcher { lattice, text, common, mtime: None, next_check: std::time::Instant::now() }
    }

    /// Returns both whole modules — every half off disk — when any of the three
    /// files changed since the last poll.
    fn poll(&mut self) -> Option<ReloadedShaders> {
        let now = std::time::Instant::now();
        if now < self.next_check {
            return None;
        }
        self.next_check = now + std::time::Duration::from_millis(500);

        let stamp = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mtime = stamp(&self.lattice)?.max(stamp(&self.text)?).max(stamp(&self.common)?);
        match self.mtime {
            None => {
                self.mtime = Some(mtime); // baseline; the baked shader is current
                None
            }
            Some(previous) if previous == mtime => None,
            Some(_) => {
                // The stamp is committed only once EVERY half is in hand. An
                // editor that saves through a temp file and a rename leaves a
                // window where the metadata is the new one and the read is not,
                // and a stamp advanced past a failed read swallows that edit
                // until the file is saved again — three times as reachable now
                // that one reload reads three files.
                let common = std::fs::read_to_string(&self.common).ok()?;
                let lattice = std::fs::read_to_string(&self.lattice).ok()?;
                let text = std::fs::read_to_string(&self.text).ok()?;
                self.mtime = Some(mtime);
                Some(ReloadedShaders {
                    common: common.clone(),
                    lattice: module_source(&common, &lattice),
                    text: module_source(&common, &text),
                    seam: common_lines(&common),
                })
            }
        }
    }
}

/// Parse + validate WGSL and check our entry points exist, so a bad edit
/// never reaches wgpu's panicking error handler. Also exercised by a unit
/// test against the baked-in source: plugin builds have no hot-reload, so
/// without that test a broken commit would first surface as a crash inside
/// a DAW at first paint.
///
/// `source` is a WHOLE module — what [`with_common`] or the watcher hands back,
/// never lattice.wgsl on its own, which names functions it does not declare and
/// would fail here for that alone. `name` is the module's own file, and
/// `required` the entry points it has to keep: the two modules validated here
/// declare different ones, and a list checked against the wrong module reports
/// every entry point in it missing.
///
/// A diagnostic's line number is the CONCATENATED module's, which is no line of
/// either file. Naga weaves those numbers through a rendered snippet, so the
/// seam is stated rather than the numbers rewritten.
///
/// `seam` is the caller's because it is a property of the common half THIS
/// `source` was built from, and the hot-reload path's is read off disk: taking
/// it from `COMMON_SRC` here would state the baked half's seam over a module
/// joined to a different one, and be wrong by however many lines common.wgsl
/// has gained or lost since the build.
#[cfg(any(test, feature = "hot-reload"))]
fn validate_wgsl(name: &str, source: &str, seam: usize, required: &[&str]) -> Result<(), String> {
    let banner = |body: String| {
        format!(
            "in {name} (lines 1-{seam} below are common.wgsl; past that, \
             subtract {seam} for the line in {name}):\n{body}"
        )
    };
    let module =
        naga::front::wgsl::parse_str(source).map_err(|e| banner(e.emit_to_string(source)))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| banner(format!("{e:?}")))?;
    for entry in required {
        if !module.entry_points.iter().any(|ep| ep.name == *entry) {
            return Err(format!("{name} is missing entry point `{entry}`"));
        }
    }
    Ok(())
}

mod uniforms;
use uniforms::*;

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
// of the `spectrum_color` array). A mismatch here is a ring reading the wrong
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
    ///
    /// Only the first THREE cross as a vertex attribute. The mark is read by
    /// the light alone, and the light is no longer drawn over this stream: it
    /// reaches the gather through [`GpuGlowNode`], which this is the source
    /// for ([`LatticeCallback::glow_nodes`]). Kept as four here because this
    /// is where a node's light is assembled, and splitting one `GlowStep`
    /// across two fields to save a float nobody uploads twice buys nothing.
    glow: [f32; 4],
}

impl GpuInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        // Locations 5 and 9 are absent, not renumbered — both are the second
        // instance-step buffer's, which rides beside this one
        // (`shadow::ShadowBox::BESIDE_NODES`). The audio ring's own slot is
        // 11, and it carries how far the layer is on at this node rather than
        // a reading: WHAT the ring says is a window onto the shared spectrum
        // in the uniforms, and how much of one this node wears is the
        // per-node half of it (`GpuInstance::ring`). The macro
        // names each location and takes each OFFSET from the sequence, so a
        // dropped entry shrinks the stride to match the struct without moving
        // the rest off their numbers — which is what keeps this list and
        // lattice.wgsl's `Instance` readable side by side.
        //
        // Location 12 is THREE of `glow`'s four floats, and the stride is still
        // the struct's: the mark is spent on the CPU into the lit-node buffer
        // and no vertex stage reads it (`GpuInstance::glow`). A narrower
        // attribute over a wider field is well-formed — the offsets above it
        // are already fixed and nothing is read past what is named.
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x4, 2 => Float32x3, 3 => Uint32x3,
            4 => Float32, 6 => Uint32x2,
            7 => Float32x4, 8 => Float32x4, 10 => Float32, 11 => Float32,
            12 => Float32x3
        ],
    };
}

/// Pack a grid of per-bucket levels into the rows `spectrum_color_level()` in
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
    /// `MarkerParams::half_width` and `MarkerParams::taper_start`.
    pos_radius: [f32; 4],
    /// rgb: the marker's own ink, a: its opacity. Both come off one resolve of
    /// `ViewConfig::marker_ink`, so a marker at rest is that grey exactly; the
    /// alpha is under one otherwise, a name claiming the position over it
    /// (`derive_pluses`).
    ///
    /// The alpha is the whole marker's and not only its ink's — the share of
    /// the shadow its cross casts is the same number
    /// (`PlusInstance::strength`), so a position handing itself over to a name
    /// hands both over together.
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
    /// GPU time of all lattice preparation passes, before the final composite
    /// in egui's own pass. Carries the
    /// [`GPU_TIME_UNSUPPORTED`] / [`GPU_TIME_PENDING`] sentinels.
    pub gpu_ms: std::sync::atomic::AtomicU32,
    /// Wall time of the whole `prepare` callback. egui-wgpu runs this from
    /// inside `update_buffers`, so it is billed to the frame's upload stage
    /// and is invisible from outside.
    ///
    /// "Prepare" undersells it, and the three fields below exist because the
    /// name misled for a long time: this callback does not merely stage data.
    /// It also encodes shadows, ink history/convolution, glow, ordered scene
    /// composition and optional bloom onto egui's encoder — CPU work in the
    /// frame, sitting inside a row the overlay calls "buf up".
    pub prepare_ms: std::sync::atomic::AtomicU32,
    /// Of that, the time in `device.poll` draining the timestamp readback:
    /// what the GPU measurement costs to take. Kept separate so the
    /// instrumentation can be caught spending the budget it exists to
    /// measure.
    pub poll_ms: std::sync::atomic::AtomicU32,
    /// Of that, staging this frame's data: sizing the offscreen targets,
    /// recreating them when the size moved, the `queue.write_buffer` calls for
    /// instances, markers, labels and both sets of uniforms, plus label-sheet
    /// binding updates, drawn-mark uploads and shadow packing.
    pub write_ms: std::sync::atomic::AtomicU32,
    /// Of that, encoding all lattice preparation passes and their draws.
    /// No GPU work happens here; this is the CPU cost of building the command
    /// stream, separate from packing, target creation and writes above.
    pub scene_ms: std::sync::atomic::AtomicU32,
}

/// `stats` receives this pane's own measurements. Pass `None` for panes whose
/// cost isn't the one being reported, so a second lattice on screen can't
/// overwrite the readings.
/// `pipeline_cache` reuses compiled handles when the shell publishes its
/// [`wgpu::Instance`] into `CallbackResources`; other shells build per window.
pub fn lattice_paint_callback(
    rect: egui::Rect,
    scene: &Scene,
    labels: LatticeLabels,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    stats: Option<std::sync::Arc<LatticeStats>>,
    pipeline_cache: std::sync::Arc<LatticePipelineCache>,
) -> egui::PaintCallback {
    let mut callback =
        LatticeCallback::from_scene(scene, labels, rect.size(), target_format, pane_id, stats);
    callback.pipeline_cache = Some(pipeline_cache);
    egui_wgpu::Callback::new_paint_callback(rect, callback)
}

/// Per-frame, per-pane draw data, computed on the UI thread.
struct LatticeCallback {
    pipeline_cache: Option<std::sync::Arc<LatticePipelineCache>>,
    instances: Vec<GpuInstance>,
    /// Row owners in exactly the shipped instance order, after sorting and culling.
    glow_owners: Vec<u64>,
    glow_timing: Option<harmonigraph_scene::GlowTiming>,
    /// Display-only breathing factors in instance order, keyed by lattice identity.
    glow_breath: Option<Vec<f32>>,
    /// Every label's glyphs, in the order the pass draws them.
    glyphs: Vec<GlyphInstance>,
    /// Every caster this frame, in the order the pass draws them: the markers'
    /// one shared cross first where the field draws any, then one per node
    /// instance and one per name, interleaved as the walk emits them. What
    /// `prepare` packs the shadow atlas from — a pure function of the frame,
    /// which the offline renderer's determinism rests on.
    casters: Vec<shadow::Caster>,
    /// Every group's style, as the scene handed it over.
    ///
    /// The casters above already carry their own σ and kernel, so what is left
    /// for this is what a group decides OUTSIDE the packing: which fill
    /// pipeline the names' cells are drawn by, and the depth a name's box
    /// spends (`fs_shadow_box` reads it out of the text pipeline's own
    /// uniform). The geometry group's pair rides in `uniforms.geometry_shadow`.
    shadow: harmonigraph_scene::ShadowSettings,
    /// Which caster each node instance's shadow is, by index into `casters` —
    /// parallel to `instances`, since the walk interleaves the two lists.
    node_cells: Vec<u32>,
    /// One arm of a resting marker in the pane's points, and 0 where the field
    /// casts nothing: what maps a fragment's place on a cross into the shared
    /// cell. The cell itself is `casters[0]` wherever this is above zero.
    marker_arm_points: f32,
    /// The scene pass's whole order, back to front — see [`Draw`].
    draws: Vec<Draw>,
    /// The fallback font sheet and drawn-mark sheet, on their publication frames.
    atlas: Option<FontAtlas>,
    marks: Option<FontAtlas>,
    sdf: Option<GlyphSdfAtlas>,
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
/// draws — the nodes, the markers and the names — is depth-sorted once, in
/// `LatticeCallback::from_scene`, and the sequence
/// falls out of that one walk. What that buys is that there is no second
/// expression of the order to keep in step with the first: a draw goes where
/// the walk put it, so nothing has to be told which side of anything else it
/// belongs on.
///
/// The alternative is what this replaces, and it is worth naming because it
/// looks cheaper. Carrying an INDEX into the node run per marker and name —
/// "how many nodes go in front of me" — costs one tiebreak per
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

impl LatticeCallback {
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
            bright: &resources.compiled.bright_pipeline,
            downsample: &resources.compiled.downsample_pipeline,
            blur_h: &resources.compiled.blur_h_pipeline,
            blur_v: &resources.compiled.blur_v_pipeline,
        }
    }
}

/// One world point through `view_proj` onto a pane of `extent` — points or
/// pixels, whichever the caller measures in — as the rasterizer would place
/// it: x right, y DOWN from the top-left corner, and wgpu's clip depth (0 near,
/// 1 far) beside it. The two places the CPU stands in for the rasterizer read
/// this: the shadow packer's boxes (`from_scene`) and the light's lit-node map
/// ([`LatticeCallback::glow_nodes`]), which have to agree with each other and
/// with `node_vertex`.
///
/// `None` for a point at or behind the eye, which no pass can place and which
/// each caller drops on its own terms.
fn project_onto(
    view_proj: &glam::Mat4,
    extent: glam::Vec2,
    p: glam::Vec3,
) -> Option<(glam::Vec2, f32)> {
    let clip = *view_proj * p.extend(1.0);
    (clip.w > 1e-4).then(|| {
        let ndc = clip / clip.w;
        (glam::vec2((ndc.x * 0.5 + 0.5) * extent.x, (0.5 - ndc.y * 0.5) * extent.y), ndc.z)
    })
}

/// Device programs, layouts and immutable fallback bindings. Cloning retains
/// GPU handles, never a context's atlas publications, pane history or timer.
/// Hot reload replaces handles only in the context's own value; no shared
/// mutable pipeline owner participates in drawing.
#[derive(Clone)]
struct CompiledLatticeResources {
    scenes: [ScenePipelines; 2],
    composite_pipeline: wgpu::RenderPipeline,
    /// Bloom chain: bright pass, half->quarter downsample, blur x2.
    bright_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    blur_h_pipeline: wgpu::RenderPipeline,
    blur_v_pipeline: wgpu::RenderPipeline,
    /// The node glow's own pass: one draw over the whole of a target of the
    /// glow's own, folding every lit node at each pixel (see
    /// [`create_glow_gather_pipeline`]).
    glow_gather_pipeline: wgpu::RenderPipeline,
    /// The lit-node list that pass walks, at its group 2 (see
    /// [`glow_node_layout`]).
    glow_node_layout: wgpu::BindGroupLayout,
    /// The colour it draws in, settled ahead of it: the ink read round every
    /// node, then blurred (see [`create_ink_strip_pipelines`]).
    ink_strip_pipeline: wgpu::RenderPipeline,
    ink_blur_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    bright_layout: wgpu::BindGroupLayout,
    /// One texture + the shared sampler, which is what every single-texture
    /// reader here binds: each pass of the bloom chain, and the glow target —
    /// taken at group 0 by the composite that lays the light down and at group
    /// 1 by the node and marker pipelines, whose washes read the same field.
    ///
    /// The glow reads its texture with `textureLoad` and the bloom samples
    /// its own, so the sampler at 1 is bound by both and spent by one. One
    /// layout rather than two of the same shape: a second would have to be
    /// kept in step with this for nothing.
    filter_layout: wgpu::BindGroupLayout,
    /// A 1x1 transparent texture in [`filter_layout`](Self::filter_layout),
    /// standing in for the glow target at group 1 wherever there is not one.
    ///
    /// Two places there is not, and neither is an error state: the Reach bar at
    /// 0 drops the target entirely (`Offscreen::ensure_glow`), and the
    /// single-attachment `fs_main`/`fs_plus` path the parity test draws through
    /// has no glow pass at all. Transparent light composites to the plain
    /// ground, so `node_paint` needs no branch for either — which is the whole
    /// reason this is a dummy texture rather than a second pipeline variant.
    glow_dummy_bind_group: wgpu::BindGroup,
    /// The same transparent texel, held for the bloom-off composite binding.
    bloom_dummy: wgpu::TextureView,
    /// One texture and NO sampler: the ink strip, which is read texel by texel
    /// (see [`InkStrip`]). Its own layout rather than `filter_layout` because
    /// the two differ in exactly that — a strip is indexed by node and by
    /// angle, and a filtered lookup across its rows would blend one node's
    /// colour into its neighbour's.
    strip_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The shadow atlas's three stages (`crate::shadow`): every name's glyphs
    /// into its cell, the passes that sweep the cells, and the box each name
    /// multiplies the scene by off its finished cell.
    glyph_coverage_cell_pipeline: wgpu::RenderPipeline,
    glyph_distance_cell_pipeline: wgpu::RenderPipeline,
    glyph_distance_pad_pipeline: wgpu::RenderPipeline,
    shadow_cell_pipelines: shadow::CellPipelines,
    /// The other two rasterizers of a cell: a node's ink into its own, and one
    /// cross into the markers' shared one (see [`create_cell_pipelines`]).
    node_cell_pipeline: wgpu::RenderPipeline,
    plus_cell_pipeline: wgpu::RenderPipeline,
    /// A 1x1 atlas in [`shadow_layout`](Self::shadow_layout), standing in at
    /// group 2 wherever this frame packed no cell — the Shadow width or depth
    /// at the bottom of its bar, or nothing in the frame that casts. Every draw
    /// then carries a box of zeros and multiplies by exactly 1, with nothing
    /// sampled, so there is no branch and no second pipeline variant.
    shadow_dummy_bind_group: wgpu::BindGroup,
    glyph_layout: wgpu::BindGroupLayout,
    /// How the atlas is read, by the blur and the box alike.
    shadow_layout: wgpu::BindGroupLayout,
    /// Every caster's kernel, at group 3 of the scene pipelines; see
    /// [`shadow::caster_layout`].
    caster_layout: wgpu::BindGroupLayout,
    glyph_sampler: wgpu::Sampler,
    blank: wgpu::Texture,
    blank_sdf: wgpu::Texture,
    target_format: wgpu::TextureFormat,
}

/// Mutable rendering state owned by one egui-wgpu callback context. Each new
/// context starts empty even when its pane IDs match a previous window.
struct LatticeResources {
    compiled: CompiledLatticeResources,
    /// This renderer's bindings for the two sheets a glyph can be cut from —
    /// egui's shared font texture and the drawn marks' private texture.
    atlas: text::AtlasTexture,
    marks: text::AtlasTexture,
    /// Identity of the shared SDF texture its glyph bind groups name. The
    /// allocation itself is stored once in `CallbackResources` and is also
    /// used by the standalone text renderer.
    sdf_key: u64,
    panes: HashMap<u64, PaneBuffers>,
    /// GPU-side timing of the lattice passes. `None` when the device didn't
    /// grant timestamp queries — plenty of GPUs (and the offline renderer,
    /// which never asks for the feature) don't, and the readout says so
    /// rather than pretending.
    timer: Option<GpuTimer>,
    #[cfg(feature = "hot-reload")]
    watcher: ShaderWatcher,
}

/// Compiled lattice pipelines retained by one UI state across editor windows.
/// Only instance, device and target format key this slot: camera, pane dimensions,
/// elapsed hidden time and drawing history do not affect compilation. The
/// cached value contains no pane targets, ink history, font atlas or timer.
/// Reopening clones GPU handles and allocates fresh window-owned mutable state.
#[derive(Default)]
pub struct LatticePipelineCache {
    // Hot reload owns live shader replacement; keep that development path's
    // existing rebuild-on-open behavior rather than caching its baked source.
    #[cfg(not(feature = "hot-reload"))]
    // wgpu compares native devices by ID, and those IDs restart per instance.
    compiled: std::sync::Mutex<Option<(wgpu::Instance, wgpu::Device, CompiledLatticeResources)>>,
    #[cfg(not(feature = "hot-reload"))]
    startup: std::sync::Mutex<startup::Initialization>,
}

impl LatticePipelineCache {
    fn resources(
        &self,
        instance: &wgpu::Instance,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> LatticeResources {
        #[cfg(feature = "hot-reload")]
        {
            let _ = instance;
            LatticeResources::new(device, queue, format)
        }
        #[cfg(not(feature = "hot-reload"))]
        {
            let mut cached = self.compiled.lock().expect("lattice pipeline cache poisoned");
            if cached.as_ref().is_none_or(|(owner_instance, owner, r)| {
                owner_instance != instance || owner != device || r.target_format != format
            }) {
                let compiled = CompiledLatticeResources::new(device, queue, format);
                *cached = Some((instance.clone(), device.clone(), compiled));
            }
            LatticeResources::from_compiled(
                cached.as_ref().expect("initialized above").2.clone(),
                device,
                queue,
            )
        }
    }
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
    /// 1x1 target for the opening and trailing timestamp passes.
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

    /// Open before any lattice preparation pass, even when optional stages skip.
    ///
    /// Both samples are BEGINNING-of-pass writes. The obvious shape —
    /// `write_timestamp` on the encoder, or beginning-and-end on one pass —
    /// does not work here: Metal advertises and grants both
    /// `TIMESTAMP_QUERY_INSIDE_ENCODERS` and end-of-pass writes, then
    /// silently records ZERO for them. Only the beginning-of-pass sample
    /// comes back with a real value, so the bracket is built from two of
    /// those, the closing one on a pass that exists only to carry it.
    fn opening(&self, encoder: &mut wgpu::CommandEncoder) {
        self.stamp(encoder, 0);
    }

    fn stamp(&self, encoder: &mut wgpu::CommandEncoder, index: u32) {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(if index == 0 {
                "lattice_gpu_timer_open_pass"
            } else {
                "lattice_gpu_timer_tail_pass"
            }),
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
                beginning_of_pass_write_index: Some(index),
                end_of_pass_write_index: None,
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    /// Close the bracket with a beginning-of-pass sample, then stage the
    /// result for a later frame to map.
    fn close(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.stamp(encoder, 1);
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
    /// Reused upload staging for coefficients resolved against encoded GPU history.
    ink_instances: Vec<GpuInstance>,
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
    /// Each node instance's own box, beside the instance buffer: the second
    /// instance-step buffer both the node draw and the cell draw bind
    /// (`shadow::ShadowBox::BESIDE_NODES`). Kept at the instance buffer's own
    /// capacity, being one row per instance.
    node_cell_buffer: wgpu::Buffer,
    node_cell_capacity: usize,
    /// Every caster's shadow, as the SCENE draws read it — one entry per caster
    /// carrying its cell and mapping ([`shadow::ShadowCaster`]), and the bind
    /// group naming it at group 3.
    ///
    /// A storage buffer rather than more rows beside the instances, for the
    /// reason [`shadow::ShadowCaster`] gives: a node's instance rows and the box
    /// beside them leave one of the sixteen attribute locations, where a caster
    /// is four vec4s. Rebuilt with the buffer, which is the one thing the
    /// atlas's own bind groups must not be — hence a group of its own
    /// (`shadow::caster_layout`).
    caster_buffer: wgpu::Buffer,
    caster_capacity: usize,
    caster_count: usize,
    caster_bind_group: wgpu::BindGroup,
    /// Every lit node this frame, as the light's own pass reads them
    /// ([`GpuGlowNode`]), and the bind group naming the buffer at group 2.
    ///
    /// Keyed on CAPACITY alone, exactly as the casters above are: the contents
    /// are rewritten every frame and neither object is, so a frame that lights
    /// one more node than the last rebuilds nothing (`glow_node_buffer`). How
    /// many of the entries are this frame's is `u.glow.lit`, and never the
    /// buffer's own length.
    glow_node_buffer: wgpu::Buffer,
    glow_node_capacity: usize,
    glow_node_bind_group: wgpu::BindGroup,
    /// Tile offsets and candidate indices, uploaded every frame. Allocation
    /// depends only on capacity; camera, reach and node changes rewrite it.
    glow_tile_buffer: wgpu::Buffer,
    glow_tile_capacity: usize,
    glow_tile_bind_group: wgpu::BindGroup,
    /// The scene pass's whole order (see [`Draw`]), held to what actually
    /// reached the buffers above.
    draws: Vec<Draw>,
    /// What the glyph shader is told about this pane: its size in points, the
    /// atlas's, and the terms a name's shadow is cast on.
    glyph_uniform_buffer: wgpu::Buffer,
    /// Names both sampled sheets, so it is rebuilt whenever either allocation
    /// is replaced — and `glyph_sheet_keys` is which bindings it names.
    glyph_bind_group: Option<wgpu::BindGroup>,
    glyph_sheet_keys: (u64, u64, u64),
    /// GPU colour history, keyed on this pane's identity and row capacity,
    /// independently of the viewport targets. A release can have no current
    /// ink, so resizing must retain these rows rather than reseeding them.
    ink_history: Option<InkStrip>,
    offscreen: Option<Offscreen>,
}

impl PaneBuffers {
    /// Called under the same drawable-geometry guard as target maintenance.
    /// Glow-off discards history; otherwise only a capacity change replaces it.
    /// A fresh strip resolves every carried row to mix = 1, reseeding from
    /// current ink. That deliberately does not preserve an inkless release on
    /// growth. Viewport size, render scale and bloom never key this history.
    fn ensure_ink_history(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        want: bool,
        rows: u32,
    ) {
        if !want {
            self.ink_history = None;
        } else if self.ink_history.as_ref().is_none_or(|strip| strip.rows != rows) {
            self.ink_history = Some(InkStrip::new(device, layout, rows));
        }
    }
}

/// The per-pane offscreen render target and bloom chain, recreated when
/// the pane's pixel size (or render scale) changes.
///
/// The scene target uses the render-scaled size; the bloom textures use
/// fractions of the pane's NATIVE screen size, so the halo's on-screen
/// width doesn't change with the render-scale setting.
struct Offscreen {
    /// The descriptor format shared by all scene attachments.
    #[cfg(test)]
    format: wgpu::TextureFormat,
    color_view: wgpu::TextureView,
    /// Node and label RGB, spared ordinary node shadows and summed at composite.
    ink_view: wgpu::TextureView,
    /// The independent label-free scene pair and its filtered halo.
    /// Present only while bloom is on; toggling it never replaces glow history.
    bloom: Option<LatticeBloom>,
    /// The node glow's own target, present only while the view asks for one.
    glow: Option<GlowTarget>,
    /// The names' shadow atlas, present only while a frame has names casting a
    /// shadow (`ensure_shadow`).
    ///
    /// Its own lifetime rather than a member of [`GlowTarget`], because the two
    /// answer to different bars: a name's shadow lands on the ground at a Reach
    /// of 0, where there is no light and no glow target at all.
    shadow: Option<shadow::ShadowTarget>,
    /// Composite: background + node ink + blurred bloom (quarter A) + uniforms.
    composite_bind_group: wgpu::BindGroup,
    size: [u32; 2],
    screen_size: [u32; 2],
}

/// The allocations whose contents are needed only while bloom is enabled.
struct LatticeBloom {
    /// The label-free background and node-ink components. Threshold their sum
    /// so separating shadow receivers does not change what counts as bright.
    nodes_view: wgpu::TextureView,
    ink_view: wgpu::TextureView,
    chain: BloomChain,
}

/// Where a frame's node light is assembled before any of it reaches the
/// picture: one transparent premultiplied colour texture at the scene's own
/// size, plus the bind group its readers take it through.
///
/// A target of its own, rather than the glow drawn straight into the scene
/// pass, because a node has to sample the finished light to paint its own
/// picture (`node_paint`), and a pass cannot sample the attachment it writes.
/// Every node's halo melds here first (`fs_glow_gather`), across every sheet at once,
/// and the scene pass then lays that one layer down at its bottom and reads it
/// again per node.
///
/// Created and dropped as the Reach bar crosses 0, independently of the resize
/// that rebuilds everything around it: the two changes have nothing to do with
/// each other, and a target left allocated at reach 0 is a scene-sized texture
/// held for a feature that is off.
struct GlowTarget {
    /// The descriptor format of `view`.
    #[cfg(test)]
    format: wgpu::TextureFormat,
    view: wgpu::TextureView,
    /// The texture + the shared sampler, as
    /// [`CompiledLatticeResources::filter_layout`] takes them.
    bind_group: wgpu::BindGroup,
}

/// A frame's ink strips: what every node is putting on itself, read round each
/// of them at [`INK_STRIP_N`] angles and blurred there.
///
/// One row per assigned glow identity, indexed by the node's `strip_row`;
/// instance order may change without changing that identity. Two raw textures
/// carry history and a third holds its convolution: `raw` is
/// `fs_ink_strip`'s reading, `blurred` is
/// `fs_ink_blur`'s convolution of it plus, in one extra column, the same
/// average at no concentration — the mean a node's middle eases toward.
///
/// Small: an f16 RGBA texel per angle per node, so a lattice of 400 lit nodes
/// spends about 600 KiB on the three textures, beside the much larger pane-sized
/// half-float attachments. That is what the light costs in memory to stop
/// costing a whole reading of the node per lit fragment.
struct InkStrip {
    history: ink_history::InkHistory,
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
    /// frame that built it, which is what [`PaneBuffers::ensure_ink_history`] compares.
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
/// hands the light. The format is explicit rather than inherited from the
/// scene because a strip texel is a normalised colour beside a WEIGHT, and a
/// weight is a layer's level times its width — small numbers that must not be
/// quantized away if the scene format changes.
const INK_STRIP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The shared, pane-independent objects an [`Offscreen`] binds against.
struct OffscreenShared<'a> {
    format: wgpu::TextureFormat,
    composite_layout: &'a wgpu::BindGroupLayout,
    bright_layout: &'a wgpu::BindGroupLayout,
    /// One texture plus the sampler; see [`CompiledLatticeResources::filter_layout`].
    filter_layout: &'a wgpu::BindGroupLayout,
    /// The shadow atlas as its readers take it; see
    /// [`CompiledLatticeResources::shadow_layout`].
    shadow_layout: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
    /// Transparent stand-in for the composite's bloom binding while off.
    bloom_dummy: &'a wgpu::TextureView,
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
    /// The descriptor format shared by all three bloom targets.
    #[cfg(test)]
    format: wgpu::TextureFormat,
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
            #[cfg(test)]
            format,
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
    ) -> Self {
        let OffscreenShared { format, .. } = *shared;
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
        let color_view = color.create_view(&Default::default());
        let ink_view = tex("lattice_offscreen_ink", size[0], size[1], format, attach_and_sample)
            .create_view(&Default::default());
        let composite_bind_group = Self::composite_binding(
            device,
            shared,
            uniform_buffer,
            &color_view,
            &ink_view,
            shared.bloom_dummy,
        );

        Offscreen {
            #[cfg(test)]
            format,
            bloom: None,
            glow: None,
            shadow: None,
            composite_bind_group,
            color_view,
            ink_view,
            size,
            screen_size,
        }
    }

    fn composite_binding(
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        uniforms: &wgpu::Buffer,
        color: &wgpu::TextureView,
        ink: &wgpu::TextureView,
        bloom: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_composite_bind_group"),
            layout: shared.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(shared.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(bloom),
                },
                wgpu::BindGroupEntry { binding: 3, resource: uniforms.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(ink),
                },
            ],
        })
    }

    /// Keyed only on enabled/disabled: sizes are fixed by this Offscreen's
    /// lifetime. A strength change within the enabled range updates uniforms
    /// alone. Neither transition touches the main target or the ink history.
    fn ensure_bloom(
        &mut self,
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        uniforms: &wgpu::Buffer,
        want: bool,
    ) {
        if want == self.bloom.is_some() {
            return;
        }
        self.bloom = want.then(|| {
            let tex = |label| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: self.size[0],
                            height: self.size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: shared.format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            let nodes_view = tex("lattice_bloom_other");
            let ink_view = tex("lattice_bloom_ink");
            let mut chain = BloomChain::new(
                device,
                "lattice",
                shared.format,
                shared.filter_layout,
                shared.sampler,
                &nodes_view,
                self.screen_size,
            );
            // Only the first step needs two sources; the shared filter tail
            // still takes one texture. Keyed by this bloom allocation's size.
            chain.bright_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lattice_bright_split"),
                layout: shared.bright_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&nodes_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(shared.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&ink_view),
                    },
                ],
            });
            LatticeBloom { nodes_view, ink_view, chain }
        });
        self.composite_bind_group = Self::composite_binding(
            device,
            shared,
            uniforms,
            &self.color_view,
            &self.ink_view,
            self.bloom.as_ref().map_or(shared.bloom_dummy, |b| &b.chain.quarter_a_view),
        );
    }

    /// Make this pane's viewport-sized light target exist while `want` says
    /// so. The separate pane history is maintained by the caller under the
    /// same guard; recreating this image never allocates or transfers a strip.
    fn ensure_glow(&mut self, device: &wgpu::Device, shared: &OffscreenShared<'_>, want: bool) {
        match (want, self.glow.is_some()) {
            (true, false) => self.glow = Some(GlowTarget::new(device, shared, self.size)),
            (false, true) => self.glow = None,
            _ => {}
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
        blurs: bool,
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
                // After the size check, so a target rebuilt just above and one
                // kept from last frame arrive at the same answer: the
                // intermediate is the atlas's own size and a rebuild drops it.
                if let Some(atlas) = self.shadow.as_mut() {
                    atlas.ensure_half(device, shared.shadow_layout, shared.sampler, blurs);
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
    /// would have to be sampled, and a filtered read of the light a node's ink
    /// is washed with is a blur nobody asked for.
    fn new(device: &wgpu::Device, shared: &OffscreenShared<'_>, size: [u32; 2]) -> Self {
        let OffscreenShared { format, filter_layout, sampler, .. } = *shared;
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("lattice_glow"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_glow_bind_group"),
            layout: filter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        GlowTarget {
            #[cfg(test)]
            format,
            view,
            bind_group,
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
    /// seeds rather than mixing: its new history has no initialized rows,
    /// whatever the last CPU layout pass supplied as its coefficient.
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, rows: u32) -> Self {
        #[cfg(test)]
        lattice_tests::INK_STRIP_CREATIONS.with(|count| count.set(count.get() + 1));
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
            history: ink_history::InkHistory::new(rows),
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

/// The three bind group layouts a scene pipeline draws through: the pane's
/// uniforms at group 0, the finished light at group 1 — the field a node washes
/// its own ink with (`node_paint`) — and the shadow atlas at group 2, the cell
/// each draw multiplies the frame by.
///
/// Both the node and the marker pipeline take all three: they are one pass over
/// one pane, so one layout is what lets the light and the atlas be bound once
/// for both. Whether there IS either to bind is the caller's business — see
/// `CompiledLatticeResources::glow_dummy_bind_group` and `shadow_dummy_bind_group`.
#[derive(Clone, Copy)]
struct SceneLayouts<'a> {
    uniforms: &'a wgpu::BindGroupLayout,
    glow: &'a wgpu::BindGroupLayout,
    shadow: &'a wgpu::BindGroupLayout,
    /// Every caster's kernel; see [`CompiledLatticeResources::caster_layout`].
    casters: &'a wgpu::BindGroupLayout,
}

/// The lattice's own shader module, out of a WHOLE module's source: what
/// [`with_common`] builds at startup, or what the watcher reads back off disk
/// on a reload. Never lattice.wgsl alone, which names what common.wgsl
/// declares.
///
/// Every pipeline cut from that text shares one module per resource build, so
/// this is the one place the text becomes a module and the one place that
/// contract is stated.
fn lattice_module(device: &wgpu::Device, shader_src: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("lattice_shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    })
}

/// Build one of the scene pipelines from the module shared by this resource
/// build. Node and marker pipelines share the module, bind group layout,
/// blending, and topology; only entry points and vertex layout differ.
///
/// `bloom` selects the second colour attachment, the independent input the
/// bright pass reads (see [`LatticeBloom::nodes_view`]). The single-attachment
/// variant serves production with bloom off and the direct parity reference.
/// Both rely on painter order and carry no depth state.
fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    layouts: SceneLayouts<'_>,
    entry_points: (&str, &str),
    vertex_layouts: &[wgpu::VertexBufferLayout<'_>],
    attachments: usize,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_pipeline_layout"),
        bind_group_layouts: &[
            Some(layouts.uniforms),
            Some(layouts.glow),
            Some(layouts.shadow),
            Some(layouts.casters),
        ],
        ..Default::default()
    });

    let color_target = wgpu::ColorTargetState {
        format: target_format,
        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    };
    let targets = vec![Some(color_target); attachments];

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        // Name the pipeline after its vertex entry point, so a GPU capture
        // can tell the node and marker passes apart.
        label: Some(entry_points.0),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(entry_points.0),
            compilation_options: Default::default(),
            buffers: vertex_layouts,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry_points.1),
            compilation_options: Default::default(),
            targets: &targets,
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

/// One target for rasterization references, two for the split visible scene,
/// four when the label-free bloom pair is also needed.
fn create_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    layouts: SceneLayouts<'_>,
    attachments: usize,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let (node, plus) = match attachments {
        1 => ("fs_main", "fs_plus"),
        2 => ("fs_main_split", "fs_plus_split"),
        4 => ("fs_main_scene", "fs_plus_scene"),
        _ => unreachable!("scene attachment count"),
    };
    (
        create_pipeline(
            device,
            shader,
            target_format,
            layouts,
            ("vs_main", node),
            &[GpuInstance::LAYOUT, shadow::ShadowBox::BESIDE_NODES],
            attachments,
        ),
        create_pipeline(
            device,
            shader,
            target_format,
            layouts,
            ("vs_plus", plus),
            &[GpuPlus::LAYOUT],
            attachments,
        ),
    )
}

/// The ordered scene pass's attachment-compatible draws. Index 0 carries
/// the two visible components; index 1 also writes the independent bloom pair.
/// Startup and hot reload build both through the same factory.
#[derive(Clone)]
struct ScenePipelines {
    nodes: wgpu::RenderPipeline,
    pluses: wgpu::RenderPipeline,
    /// Ink washed by the light at group 1, writing only the visible picture.
    glyph_fill: wgpu::RenderPipeline,
    /// Each label's shadow immediately precedes its ink in painter order.
    shadow_box: wgpu::RenderPipeline,
    glow_over: wgpu::RenderPipeline,
}

fn create_scene_pipelines(
    device: &wgpu::Device,
    lattice_shader: &wgpu::ShaderModule,
    blit_shader: &wgpu::ShaderModule,
    glyph_shader: &wgpu::ShaderModule,
    layouts: SceneLayouts<'_>,
    glyph_layout: &wgpu::BindGroupLayout,
) -> [ScenePipelines; 2] {
    [false, true].map(|bloom| {
        let (nodes, pluses) = create_pipelines(
            device,
            lattice_shader,
            LATTICE_COLOR_FORMAT,
            layouts,
            if bloom { 4 } else { 2 },
        );
        ScenePipelines {
            nodes,
            pluses,
            glyph_fill: text::create_text_pipeline(
                device,
                glyph_shader,
                LATTICE_COLOR_FORMAT,
                glyph_layout,
                Some(layouts),
                ("vs_glyph_lit", "fs_fill_lit"),
                if bloom { 4 } else { 2 },
                EGUI_BLEND,
            ),
            shadow_box: text::create_shadow_box_pipeline(
                device,
                glyph_shader,
                glyph_layout,
                layouts.glow,
                layouts.shadow,
                layouts.casters,
                LATTICE_COLOR_FORMAT,
                bloom,
            ),
            glow_over: create_glow_over_pipeline(
                device,
                blit_shader,
                LATTICE_COLOR_FORMAT,
                layouts.glow,
                bloom,
            ),
        }
    })
}

/// The two draws that FILL the shadow atlas, from one source: a node's own ink
/// into that node's cell, and one cross into the markers' shared one.
///
/// Group 0 alone. Neither may bind the atlas — a texture cannot be read while
/// it is the target being written — so both take its size off `u.shadow_target`
/// instead, and neither reads the light: what a cell holds is coverage, and the
/// colour it is laid down in is settled where the cell is READ.
///
/// The node pipeline overwrites its cell so a negative analytic interior
/// survives the target's zero clear; one node owns one cell. The marker
/// pipeline is MAX-blended like the glyph pass beside it
/// (`text::create_glyph_cell_pipeline`) so repeated ink forms one union.
fn create_cell_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    uniforms: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    const MAX_COMPONENT: wgpu::BlendComponent = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Max,
    };
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_cell_pipeline_layout"),
        bind_group_layouts: &[Some(uniforms)],
        ..Default::default()
    });
    let pipeline = |entries: (&str, &str),
                    buffers: &[wgpu::VertexBufferLayout<'_>],
                    blend: Option<wgpu::BlendState>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entries.0),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(entries.0),
                compilation_options: Default::default(),
                buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(entries.1),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: shadow::ATLAS_FORMAT,
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
    };
    (
        pipeline(
            ("vs_node_cell", "fs_node_cell"),
            &[GpuInstance::LAYOUT, shadow::ShadowBox::BESIDE_NODES],
            // One node owns one cell. Overwrite lets an analytic distance keep
            // its negative interior; MAX against the clear value would clamp
            // every inside texel back to zero.
            None,
        ),
        // No instance data at all: the cell is one cross at the home sheet's
        // size, and what varies between markers is spent where it is read.
        pipeline(
            ("vs_plus_cell", "fs_plus_cell"),
            &[],
            Some(wgpu::BlendState { color: MAX_COMPONENT, alpha: MAX_COMPONENT }),
        ),
    )
}

/// The node glow's one pipeline: the light, gathered over the whole of the
/// glow's own target (see [`GlowTarget`]).
///
/// **One quad and no instances.** The nodes arrive as a read-only storage
/// buffer at group 2 ([`glow_node_buffer`]); group 3 narrows the walk to each
/// tile's candidates. That is the change #680 is built on: an operator written in
/// shader code is not confined to what a fixed-function blend can express.
///
/// **NO BLEND**, where a billboard per node needed one.
/// `fs_glow_gather` combines luminance with peak-normalized screen and mixes
/// colour separately. The fixed full-strength ceiling comes from Glow gain;
/// notes and their fades never move it. A lone glow keeps its original colour
/// and coverage. The light remains independent of instance order.
/// Glow accumulation crossfades to the original per-channel screen in this
/// same pass, deliberately relaxing the fixed ceiling as its share rises.
///
/// **Every sheet at once**, which is what the fold's commutativity buys as it
/// bought it for the blend. What occludes a node's halo is the scene pass,
/// which draws every node over the finished light — its SHAPE, at least: what
/// the node's own ink then takes of the light under it is `node_paint`'s to
/// say.
///
/// **No depth.** The pass this draws into carries none: it is the glow's own,
/// ahead of the scene's, and one write per pixel has no order to defend.
fn create_glow_gather_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    strip_layout: &wgpu::BindGroupLayout,
    node_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_glow_pipeline_layout"),
        bind_group_layouts: &[
            Some(bind_group_layout),
            Some(strip_layout),
            Some(node_layout),
            Some(node_layout),
        ],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fs_glow_gather"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_glow_gather"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_glow_gather"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
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
    shader: &wgpu::ShaderModule,
    bind_group_layout: &wgpu::BindGroupLayout,
    strip_layout: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let build = |label: &str,
                 entry_points: (&str, &str),
                 layout: &wgpu::PipelineLayout,
                 buffers: &[wgpu::VertexBufferLayout<'_>]| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(entry_points.0),
                compilation_options: Default::default(),
                buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
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
/// With bloom on it writes both the picture and the independent bloom input.
/// With bloom off it writes just the visible pair.
/// Both use painter order and the same premultiplied blend.
fn create_glow_over_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    light_layout: &wgpu::BindGroupLayout,
    bloom: bool,
) -> wgpu::RenderPipeline {
    // The light alone: this pass lays a finished field down and takes no
    // dial off the scene's uniforms.
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_glow_over_pipeline_layout"),
        bind_group_layouts: &[Some(light_layout)],
        ..Default::default()
    });
    let target = wgpu::ColorTargetState {
        format: target_format,
        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    };
    let targets = vec![Some(target); if bloom { 4 } else { 2 }];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fs_glow_over"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_blit"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if bloom { "fs_glow_over" } else { "fs_glow_split" }),
            compilation_options: Default::default(),
            targets: &targets,
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

/// The fullscreen shader shared by one resource build's post-process and glow
/// pipelines. Not named for the lattice: the roll builds its own pipelines out
/// of the same source, so a validation error carrying the lattice's name would
/// send a reader to the wrong picture.
fn blit_module(device: &wgpu::Device) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit_shader"),
        source: wgpu::ShaderSource::Wgsl(BLIT_SRC.into()),
    })
}

/// One post-process pipeline over the shared blit module: a fullscreen quad
/// with the given fragment entry point. The composite (into the egui pass)
/// blends premultiplied; the bloom-chain passes overwrite their whole target
/// and pass `blend: None`.
fn create_post_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    entry_point: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("post_pipeline_layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry_point),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_blit"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
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

impl CompiledLatticeResources {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, target_format: wgpu::TextureFormat) -> Self {
        Self::new_with_progress(device, queue, target_format, |_| {})
    }

    fn new_with_progress(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        progress: impl Fn(startup::Stage),
    ) -> Self {
        progress(startup::Stage::Shapes);
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
        // Ahead of the scene pipelines because they take both at groups 1 and
        // 2: a node washes its own ink with the light the pass has just
        // composited, and multiplies the frame under it by its own cell of the
        // atlas.
        let shadow_layout = shadow::read_layout(device);
        let caster_layout = shadow::caster_layout(device);
        // The whole module, common half and all, built once for every pipeline
        // cut from it.
        let shader_src = with_common(SHADER_SRC);
        let lattice_shader = lattice_module(device, &shader_src);
        let blit_shader = blit_module(device);
        let (node_cell_pipeline, plus_cell_pipeline) =
            create_cell_pipelines(device, &lattice_shader, &bind_group_layout);
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
        let glow_node_layout = glow_node_layout(device);
        progress(startup::Stage::Lighting);
        let glow_gather_pipeline = create_glow_gather_pipeline(
            device,
            &lattice_shader,
            LATTICE_COLOR_FORMAT,
            &bind_group_layout,
            &strip_layout,
            &glow_node_layout,
        );
        let (ink_strip_pipeline, ink_blur_pipeline) =
            create_ink_strip_pipelines(device, &lattice_shader, &bind_group_layout, &strip_layout);

        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_composite_bind_group_layout"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                texture_entry(2),
                uniform_entry(3),
                texture_entry(5),
            ],
        });
        progress(startup::Stage::Bloom);
        let composite_pipeline = create_post_pipeline(
            device,
            &blit_shader,
            "fs_composite",
            target_format,
            &composite_layout,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let filter = |entry| {
            create_post_pipeline(
                device,
                &blit_shader,
                entry,
                LATTICE_COLOR_FORMAT,
                &filter_layout,
                None,
            )
        };
        let bright_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_bright_layout"),
            entries: &[texture_entry(0), sampler_entry(1), texture_entry(5)],
        });
        let bright_pipeline = create_post_pipeline(
            device,
            &blit_shader,
            "fs_bright_split",
            LATTICE_COLOR_FORMAT,
            &bright_layout,
            None,
        );
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

        // The label pipelines share both attachment choices with the scene,
        // preserving each label's place in painter order.
        let glyph_layout = text::glyph_bind_group_layout(device);
        // Compiled once for the three pipelines below, as `shader_src` is for
        // the lattice's.
        let glyph_shader = text::glyph_shader(device, &text_source());
        progress(startup::Stage::Lattice);
        let scenes = create_scene_pipelines(
            device,
            &lattice_shader,
            &blit_shader,
            &glyph_shader,
            SceneLayouts {
                uniforms: &bind_group_layout,
                glow: &filter_layout,
                shadow: &shadow_layout,
                casters: &caster_layout,
            },
            &glyph_layout,
        );
        progress(startup::Stage::Labels);
        let (
            glyph_coverage_cell_pipeline,
            glyph_distance_cell_pipeline,
            glyph_distance_pad_pipeline,
        ) = text::create_glyph_cell_pipelines(device, &glyph_shader, &glyph_layout);
        progress(startup::Stage::Shadows);
        let shadow_cell_pipelines = shadow::create_cell_pipelines(device, &shadow_layout);
        progress(startup::Stage::Interface);

        // The stand-in light: one transparent texel. It is the format the real
        // target is in so that one bind group layout serves both, and ONE texel
        // because `node_paint` clamps its read into the texture's bounds — so
        // every fragment on screen reads this same nothing, whatever its
        // coordinate, which is exactly what "no light here" means. The clamp
        // is the shader's and not the backend's: WGSL lets an out-of-bounds
        // `textureLoad` answer (0,0,0,1) as readily as zero, and an alpha of
        // 1 here is every wash laid over black.
        //
        // RENDER_ATTACHMENT alongside the binding though nothing ever draws
        // into it: that usage is what gives wgpu a way to zero-initialize the
        // texture, and a zero it cannot write is a texel of whatever the
        // driver left there smeared under every node's ink.
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
        let glow_dummy = stand_in("lattice_glow_dummy", LATTICE_COLOR_FORMAT);
        let glow_dummy_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_glow_dummy_bind_group"),
            layout: &filter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&glow_dummy),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        // The stand-in atlas, on the same terms: a frame with no caster packs
        // no cell, and every draw then carries a box of zeros — which
        // `shadow_through` reads as a caster with no cell and leaves the frame
        // exactly whole, with nothing sampled. The binding still has to be
        // FILLED, so this is what fills it.
        let shadow_dummy = stand_in("lattice_shadow_dummy", shadow::ATLAS_FORMAT);
        let shadow_dummy_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_shadow_dummy_bind_group"),
            layout: &shadow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_dummy),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        Self {
            scenes,
            composite_pipeline,
            bright_pipeline,
            downsample_pipeline,
            blur_h_pipeline,
            blur_v_pipeline,
            glow_gather_pipeline,
            glow_node_layout,
            ink_strip_pipeline,
            ink_blur_pipeline,
            bind_group_layout,
            composite_layout,
            bright_layout,
            filter_layout,
            glow_dummy_bind_group,
            bloom_dummy: glow_dummy,
            strip_layout,
            sampler,
            glyph_coverage_cell_pipeline,
            glyph_distance_cell_pipeline,
            glyph_distance_pad_pipeline,
            shadow_cell_pipelines,
            node_cell_pipeline,
            plus_cell_pipeline,
            shadow_dummy_bind_group,
            shadow_layout,
            caster_layout,
            glyph_layout,
            glyph_sampler: text::glyph_sampler(device),
            blank: text::blank_atlas(device, queue),
            blank_sdf: text::blank_sdf_atlas(device, queue),
            target_format,
        }
    }
}

impl LatticeResources {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, target_format: wgpu::TextureFormat) -> Self {
        Self::from_compiled(
            CompiledLatticeResources::new(device, queue, target_format),
            device,
            queue,
        )
    }

    fn from_compiled(
        compiled: CompiledLatticeResources,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Self {
        Self {
            compiled,
            atlas: text::AtlasTexture::default(),
            marks: text::AtlasTexture::default(),
            sdf_key: 0,
            panes: HashMap::new(),
            timer: GpuTimer::new(device, queue),
            #[cfg(feature = "hot-reload")]
            watcher: ShaderWatcher::new(),
        }
    }

    /// Bind egui's current font texture and upload whichever fallback sheet moved.
    ///
    /// The text callback answers the same question with a great deal more
    /// (`text::TextResources::bind_sheets`, which carries every pane already
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
    fn bind_sheets(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shared_atlas: Option<&wgpu::Texture>,
        fallback_atlas: Option<&FontAtlas>,
        marks: Option<&FontAtlas>,
        sdf_key: u64,
    ) {
        if let Some(atlas) = fallback_atlas.filter(|a| !self.atlas.holds(a)) {
            self.atlas.upload(device, queue, atlas);
        } else if let Some(atlas) = shared_atlas {
            self.atlas.share(atlas);
        }
        if let Some(marks) = marks.filter(|a| !self.marks.holds(a)) {
            self.marks.upload(device, queue, marks);
        }
        self.sdf_key = sdf_key;
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
        sdf: Option<&wgpu::Texture>,
    ) -> &mut PaneBuffers {
        let layout = &self.compiled.bind_group_layout;
        let caster_layout = &self.compiled.caster_layout;
        let node_layout = &self.compiled.glow_node_layout;
        // Taken before the pane is borrowed: the view is a fresh handle onto
        // this frame's font texture and mark sheet — `prepare` binds them
        // before it gets here.
        let (glyph_layout, glyph_sampler) =
            (&self.compiled.glyph_layout, &self.compiled.glyph_sampler);
        let atlas_view = self.atlas.view();
        let mark_view = self.marks.view_or(&self.compiled.blank);
        let sdf_view = sdf.unwrap_or(&self.compiled.blank_sdf).create_view(&Default::default());
        let sheet_keys = (self.atlas.key(), self.marks.key(), self.sdf_key);
        let shared = OffscreenShared {
            format: LATTICE_COLOR_FORMAT,
            composite_layout: &self.compiled.composite_layout,
            bright_layout: &self.compiled.bright_layout,
            filter_layout: &self.compiled.filter_layout,
            shadow_layout: &self.compiled.shadow_layout,
            sampler: &self.compiled.sampler,
            bloom_dummy: &self.compiled.bloom_dummy,
        };
        let want_casters = wants.casters;
        let want_glow_nodes = wants.glow_nodes;
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
            let (caster_buffer, caster_bind_group) =
                shadow::caster_buffer(device, caster_layout, INITIAL_BOX_CAPACITY);
            let (glow_node_buffer, glow_node_bind_group) =
                glow_node_buffer(device, node_layout, INITIAL_GLOW_NODE_CAPACITY);
            let (glow_tile_buffer, glow_tile_bind_group) =
                shadow::storage_list::<u32>(device, node_layout, 1, "lattice_glow_tiles");
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
                ink_instances: Vec::new(),
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
                node_cell_buffer: create_vertex_buffer::<shadow::ShadowBox>(
                    device,
                    "lattice_node_cells",
                    INITIAL_INSTANCE_CAPACITY,
                ),
                node_cell_capacity: INITIAL_INSTANCE_CAPACITY,
                caster_buffer,
                caster_capacity: INITIAL_BOX_CAPACITY,
                caster_count: 0,
                caster_bind_group,
                glow_node_buffer,
                glow_node_capacity: INITIAL_GLOW_NODE_CAPACITY,
                glow_node_bind_group,
                glow_tile_buffer,
                glow_tile_capacity: 1,
                glow_tile_bind_group,
                draws: Vec::new(),
                glyph_uniform_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("lattice_glyph_uniforms"),
                    size: std::mem::size_of::<text::TextUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                glyph_bind_group: None,
                glyph_sheet_keys: (u64::MAX, u64::MAX, u64::MAX),
                ink_history: None,
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
                    &sdf_view,
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
                // Release the large old images before allocating replacements,
                // as before. The pane-owned ink history stays alive beside them.
                pane.offscreen = None;
                pane.offscreen =
                    Some(Offscreen::new(device, &shared, &pane.uniform_buffer, size, screen_size));
            }
            // Empty geometry skips both decisions, as before: this is target
            // maintenance, not a hidden-view lifecycle or retirement policy.
            pane.ensure_ink_history(device, &self.compiled.strip_layout, wants.glow, wants.rows);
            if let Some(offscreen) = pane.offscreen.as_mut() {
                offscreen.ensure_glow(device, &shared, wants.glow);
                offscreen.ensure_shadow(device, &shared, wants.shadow, wants.blurs);
            }
        }
        // Empty frames still retire disabled bloom, but keep enabled targets
        // through silence: the next note should not allocate them all again.
        // First allocation waits until a scene pass can actually write them.
        if let Some(offscreen) =
            pane.offscreen.as_mut().filter(|_| !wants.bloom || offscreen_size.is_some())
        {
            offscreen.ensure_bloom(device, &shared, &pane.uniform_buffer, wants.bloom);
        }
        // The casters' kernels, whose buffer and bind group are one object:
        // rebuilt together or the group names a buffer that is gone.
        if want_casters > pane.caster_capacity {
            pane.caster_capacity = want_casters.next_power_of_two();
            let (buffer, bind_group) =
                shadow::caster_buffer(device, caster_layout, pane.caster_capacity);
            pane.caster_buffer = buffer;
            pane.caster_bind_group = bind_group;
        }
        // The lit nodes, on exactly the same terms: one object in two halves,
        // grown when a frame lights more of them than the buffer holds and left
        // alone otherwise. Never shrunk — a chord released is a chord about to
        // be played again, and the light's own release outlives the notes.
        if want_glow_nodes > pane.glow_node_capacity {
            pane.glow_node_capacity = want_glow_nodes.next_power_of_two();
            let (buffer, bind_group) =
                glow_node_buffer(device, node_layout, pane.glow_node_capacity);
            pane.glow_node_buffer = buffer;
            pane.glow_node_bind_group = bind_group;
        }
        if wants.glow_tiles > pane.glow_tile_capacity {
            pane.glow_tile_capacity = wants.glow_tiles.next_power_of_two();
            (pane.glow_tile_buffer, pane.glow_tile_bind_group) = shadow::storage_list::<u32>(
                device,
                node_layout,
                pane.glow_tile_capacity,
                "lattice_glow_tiles",
            );
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

/// And for the light's own node list: one per LIT node, which a chord's worth
/// of release keeps well under the instance count.
const INITIAL_GLOW_NODE_CAPACITY: usize = 64;

/// Which of a pane's optional targets this frame wants, and how tall the ink
/// strip has to be — the answers `pane_buffers` acts on that come off the
/// VIEW rather than off the pane's pixels.
///
/// Together because they are one question asked
/// once per frame: what does this view need allocated. The pixels beside them
/// (`offscreen_size`, `screen_size`) are a different question and stay separate.
struct PaneTargets {
    bloom: bool,
    /// The node light, which the Reach bar switches (`Offscreen::ensure_glow`).
    glow: bool,
    /// The names' shadow atlas, at the size this frame's cells pack to, or
    /// none where no name casts one (`Offscreen::ensure_shadow`).
    shadow: Option<[u32; 2]>,
    /// Whether any cell this frame packed holds COVERAGE, and so whether the
    /// blur's intermediate is held beside the atlas
    /// (`shadow::ShadowTarget::ensure_half`). A frame whose every group answers
    /// a distance leaves it `false` and allocates one plane instead of two.
    blurs: bool,
    /// The ink strip's height: the row map's own capacity.
    rows: u32,
    /// How many casters this frame's kernels are packed for — the storage
    /// buffer at group 3 and the bind group naming it, which grow together
    /// (`shadow::caster_buffer`). Sized HERE rather than beside the vertex
    /// buffers below, because a storage buffer's bind group has to be rebuilt
    /// with it and this is where the layout is in scope.
    casters: usize,
    /// And how many lit nodes the light's own pass walks — the storage buffer
    /// at its group 2 and the bind group naming it, which grow together
    /// ([`glow_node_buffer`]), here for the same reason the casters are.
    glow_nodes: usize,
    glow_tiles: usize,
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

/// Fill native staging directly; dropping the view schedules its copy. Byte
/// arrays require no typed alignment, and `write_iter` enforces the exact row
/// count. wgpu still allocates its staging buffer for each nonempty write.
fn write_shadow_boxes(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    count: usize,
    boxes: impl Iterator<Item = shadow::ShadowBox>,
) {
    const CELL_BYTES: usize = std::mem::size_of::<shadow::ShadowBox>();
    let Some(size) = wgpu::BufferSize::new((count * CELL_BYTES) as u64) else {
        assert_eq!(boxes.count(), 0, "empty shadow upload must have no cells");
        return;
    };
    let mut view = queue.write_buffer_with(buffer, 0, size).expect("valid shadow upload");
    let (rows, tail) = view.slice(..).into_chunks::<CELL_BYTES>();
    debug_assert!(tail.is_empty());
    rows.write_iter(boxes.map(bytemuck::cast::<_, [u8; CELL_BYTES]>));
}

mod gpu_harness;
// Dependent crates exercise the same GPU requirement as this crate's tests.
#[doc(hidden)]
pub use gpu_harness::{headless_device as test_gpu_device, test_gpu_adapter};
#[cfg(test)]
mod lattice_tests;
