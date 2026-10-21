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
//! while bloom is on the pass carries one more colour attachment without
//! their ink or coverage, and the bright pass reads that.
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

mod bloom_chain;
mod gpu_timer;
mod ink_history;
mod lattice_frame;
mod lattice_pipelines;
mod lattice_prepare;
mod lattice_targets;
mod pass_aged;

use bloom_chain::*;
use gpu_timer::GpuTimer;
#[cfg(test)]
use gpu_timer::TimerState;
pub use gpu_timer::{GPU_TIME_INACTIVE, GPU_TIME_PENDING, GPU_TIME_UNSUPPORTED};
use lattice_pipelines::*;
use lattice_targets::*;

pub mod gpu_memory;
pub mod shader_assets;
/// Progress for interactive editor initialization; offline rendering remains synchronous.
pub mod startup;

/// The piano roll's own callback — a different picture with the same
/// problem, solved the same way. It shares this crate's wgpu version, buffer
/// helpers and [`BloomChain`]; the lattice's offscreen target
/// are beside the point for a flat ribbon.
mod roll;
pub use roll::{
    roll_paint_callback_with_clipped_tail, roll_surround_reach, RollAxes, RollCut, RollInstance,
    ROLL_CUT_REACH_RANGE, ROLL_HALO_BLEND_MAX,
};

/// The spectrogram's heatmap — the pane's other heavy layer, and the one whose
/// picture a CPU compose would build texel by texel. Here the aggregator's slab
/// grid is the texture and the read is per fragment, so a zoom, a resize or a
/// palette change is a uniform.
mod spectrogram;
pub use spectrogram::{
    spectrogram_paint_callback, SpectrogramAtmosphere, SpectrogramGrid, SpectrogramHeadless,
    SpectrogramRead, SpectrogramShades, SpectrogramSlab, SpectrogramVertex,
};

mod dot_shadow;
/// A halo alone, over marks a pane drew for itself — the third caller of
/// [`BloomChain`], and the one that draws no picture of its own.
mod glow;
mod lattice_material;
mod lattice_node_glow;
mod lattice_stars;
mod stars;
pub use dot_shadow::dot_shadow_paint_callback;
pub use glow::{glow_paint_callback, GlowDot};
use lattice_node_glow::create_glow_pipelines;

/// Label text, for the same reason the roll has its own callback: what a
/// label costs is the rim, and the rim was the text drawn again once per
/// stamp.
mod text;
pub use text::{
    text_paint_callback, FontAtlas, GlyphInstance, GlyphRuns, PaneIds, SheetUploads, SlideAxis,
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
/// offscreen target is sized at `ViewConfig::render_scale`, so text drawn into it
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
/// the lattice, the piano roll and the spiral's dots all read their respective
/// strengths through it. Their UI controls are independent; the renderer's
/// defensive limit remains wider than either control's range.
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
/// four-attachment form with bloom; `_split` writes the visible pair and local
/// transmission; the bare pair is
/// the single-attachment one the parity test's reference path uses; the
/// `glow` entry points rasterize directional halos and resolve overlap statistics;
/// the `ink` four
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
    "vs_glow_splat",
    "fs_glow_splat",
    "vs_glow_resolve",
    "fs_glow_resolve",
    "vs_source_shadow",
    "fs_source_shadow",
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
// lattice.wgsl clamps the span to its own literal MAX_SPAN.
const _: () = assert!(harmonigraph_scene::MAX_SPAN == 11);

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
    /// x: activation, y: melody mark level, z: bass mark level (see
    /// lattice.wgsl). The first three are levels the same node
    /// draws at, read together by the layers that draw it.
    /// The fourth component is reserved for shadow-cell kinds in cell draws.
    params: [f32; 4],
    /// Per-octave activation, 8 bits per slot, little-endian packed
    /// (slot 0 = lowest byte of the first word).
    octaves: [u32; 3],
    /// Three 10-bit poses per word, indexed by displayed slice (one spare).
    motion: [u32; 4],
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
    /// Carried brightness, stable ink row, and the renderer's color-history
    /// coefficient, followed by display-only breathing. Untimed snapshots
    /// seed current ink with coefficient 1 and no breathing modulation.
    glow: [f32; 4],
    /// How thick each octave's lit slice is drawn (`NodeInstance::thickness`),
    /// laid out as [`octaves`](Self::octaves) is on a scale of its own
    /// ([`pack_thickness`]).
    thickness: [u32; 3],
}

impl GpuInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        // Location 1 is the slices' thickness. Locations 5 and 9 belong to
        // the second instance-step buffer, which rides beside this one
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
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x3, 2 => Float32x4, 3 => Uint32x3, 15 => Uint32x4,
            4 => Float32, 6 => Uint32x2,
            7 => Float32x4, 8 => Float32x4, 10 => Float32, 11 => Float32,
            12 => Float32x4, 1 => Uint32x3
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

/// Pack the per-octave activation levels, 0..1, into the bit layout
/// `octave_level()` in lattice.wgsl unpacks: 8 bits per slot, little-endian
/// (slot 0 = lowest byte of the first word).
fn pack_octaves(levels: &[f32; harmonigraph_scene::OCTAVE_SLOTS]) -> [u32; 3] {
    pack_slot_bytes(levels, 255.0)
}

/// How many codes of a thickness byte make one band width, which
/// `slice_thickness()` in lattice.wgsl divides back out. 1 lands on a code of
/// its own, so a slice at rest unpacks to exactly 1 and the shader takes its
/// full-slice path there; the byte's top, 255, is a hair short of the 4 that
/// `THICKNESS_MAX_RANGE` ends on, the one value it rounds.
const THICKNESS_STEPS: f32 = 64.0;

/// Pack the slices' thickness, as multiples of the band's width, in the
/// layout [`pack_octaves`] uses and on [`THICKNESS_STEPS`]' scale.
fn pack_thickness(thickness: &[f32; harmonigraph_scene::OCTAVE_SLOTS]) -> [u32; 3] {
    pack_slot_bytes(thickness, THICKNESS_STEPS)
}

/// Unpack one slot of [`pack_thickness`], as the shader does.
fn unpack_thickness(words: &[u32; 3], slot: usize) -> f32 {
    ((words[slot / 4] >> ((slot % 4) * 8)) & 0xFF) as f32 / THICKNESS_STEPS
}

/// A byte a slot, `steps` codes to the unit, clamped to the byte.
fn pack_slot_bytes(values: &[f32; harmonigraph_scene::OCTAVE_SLOTS], steps: f32) -> [u32; 3] {
    let mut words = [0u32; 3];
    for (slot, &value) in values.iter().enumerate() {
        let byte = (value * steps).round().clamp(0.0, 255.0) as u32;
        words[slot / 4] |= byte << ((slot % 4) * 8);
    }
    words
}

/// How far past the band's outer edge a node's farthest lit slice swells, in
/// uv, off the packed `thickness` the shader reads: 0 where none does. A
/// bound for sizing and culling, `slices_outer` in lattice.wgsl without the cap
/// it keeps inside the billboard.
fn swelled_past_band(scene: &Scene, thickness: &[u32; 3]) -> f32 {
    let width = scene.outer_outer - scene.outer_inner;
    let t = (0..harmonigraph_scene::OCTAVE_SLOTS)
        .map(|slot| unpack_thickness(thickness, slot))
        .fold(1.0, f32::max);
    if width > 0.0 && t > 1.0 {
        width * (t - 1.0)
    } else {
        0.0
    }
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
    /// The last caster at this cross's own position in the walk — its node's
    /// or its name's, or the last before it where it stands alone. The rings
    /// that hide it are the node casters past this (`node_visibility`), so
    /// the ring of the node it stands under never does.
    place: u32,
}

impl GpuPlus {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuPlus>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Uint32],
    };
}

/// Build the egui shape that renders `scene` into `rect`. `pane_id` must be
/// unique per lattice view shown in the same frame (each gets its own GPU
/// buffers; the pipeline is shared).
/// Where the lattice callback publishes what it measures about itself.
///
/// An atomic bag rather than return values because none of this comes back up
/// the call stack that asked for it: `prepare` runs inside egui-wgpu, and the
/// GPU timing arrives several frames after the frame it describes. Values
/// are f32 bits, consumed once by the performance overlay.
pub struct LatticeStats {
    /// GPU time of all lattice preparation passes, before the final composite
    /// in egui's own pass. Carries the
    /// [`GPU_TIME_UNSUPPORTED`] / [`GPU_TIME_PENDING`] / [`GPU_TIME_INACTIVE`] sentinels.
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
    /// NaN means no callback has published since the previous consumption.
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

impl Default for LatticeStats {
    fn default() -> Self {
        use std::sync::atomic::AtomicU32;
        Self {
            gpu_ms: AtomicU32::new(GPU_TIME_PENDING),
            prepare_ms: AtomicU32::new(f32::NAN.to_bits()),
            poll_ms: AtomicU32::new(0),
            write_ms: AtomicU32::new(0),
            scene_ms: AtomicU32::new(0),
        }
    }
}

/// `stats` receives this pane's own measurements. Pass `None` for panes whose
/// cost isn't the one being reported, so a second lattice on screen can't
/// overwrite the readings.
/// `pipeline_cache` reuses compiled handles when the shell publishes its
/// [`wgpu::Instance`] into `CallbackResources`; other shells build per window.
/// `frames` lends the vectors the frame is built in and takes them back once
/// it has drawn.
#[allow(clippy::too_many_arguments)]
pub fn lattice_paint_callback(
    rect: egui::Rect,
    scene: &Scene,
    labels: LatticeLabels,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    stats: Option<std::sync::Arc<LatticeStats>>,
    pipeline_cache: std::sync::Arc<LatticePipelineCache>,
    frames: std::sync::Arc<LatticeFrames>,
) -> egui::PaintCallback {
    let mut callback = LatticeCallback::from_scene_in(
        frames.lend(),
        scene,
        labels,
        rect.size(),
        target_format,
        pane_id,
        stats,
    );
    callback.pipeline_cache = Some(pipeline_cache);
    callback.frames = Some(frames);
    egui_wgpu::Callback::new_paint_callback(rect, callback)
}

/// The vectors lattice frames are built in, handed back by each frame's
/// callback once egui has drawn and dropped it, and refilled by the next
/// frame rather than allocated afresh: at the node cap they come to several
/// megabytes a frame. One per window; each lattice pane on it takes a set.
///
/// A set keeps the largest frame it has held and is never shrunk: at most one
/// frame at the node cap per pane, which is what each frame allocated anyway,
/// and what the allocator went on holding once it was freed.
#[derive(Default)]
pub struct LatticeFrames(std::sync::Mutex<Vec<lattice_frame::FrameVecs>>);

impl LatticeFrames {
    /// More sets than a window has lattice panes is memory nothing reuses.
    const KEPT: usize = 4;

    fn lend(&self) -> lattice_frame::FrameVecs {
        self.0.lock().ok().and_then(|mut sets| sets.pop()).unwrap_or_default()
    }

    fn give_back(&self, set: lattice_frame::FrameVecs) {
        if let Ok(mut sets) = self.0.lock() {
            if sets.len() < Self::KEPT {
                sets.push(set);
            }
        }
    }
}

/// Per-frame, per-pane draw data, computed on the UI thread.
struct LatticeCallback {
    pipeline_cache: Option<std::sync::Arc<LatticePipelineCache>>,
    instances: Vec<GpuInstance>,
    /// Row owners in exactly the shipped instance order, after sorting and culling.
    glow_owners: Vec<u64>,
    glow_timing: Option<harmonigraph_scene::GlowTiming>,
    glow_blend: f32,
    material: lattice_material::Material,
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
    sheets: text::SheetUploads,
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
    /// Where this frame's vectors go back to once it is dropped, and the
    /// walk's scratch, carried along so it goes back with them.
    frames: Option<std::sync::Arc<LatticeFrames>>,
    walk: lattice_frame::Walk,
}

impl Drop for LatticeCallback {
    fn drop(&mut self) {
        let Some(frames) = &self.frames else { return };
        use std::mem::take;
        frames.give_back(lattice_frame::FrameVecs {
            instances: take(&mut self.instances),
            glow_owners: take(&mut self.glow_owners),
            glyphs: take(&mut self.glyphs),
            casters: take(&mut self.casters),
            node_cells: take(&mut self.node_cells),
            draws: take(&mut self.draws),
            pluses: take(&mut self.pluses),
            walk: take(&mut self.walk),
        });
    }
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
/// 1 far) beside it. The shadow packer's boxes use this projection and must
/// agree with `node_vertex`. Halo quads use the rasterizer directly.
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
    shaders: ShaderPipelines,
    composite_pipeline: wgpu::RenderPipeline,
    /// Bloom chain: bright pass, half->quarter downsample, blur x2.
    bright_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    blur_h_pipeline: wgpu::RenderPipeline,
    blur_v_pipeline: wgpu::RenderPipeline,
    glow_statistics_layout: wgpu::BindGroupLayout,
    material: lattice_material::Pipelines,
    bind_group_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    /// One texture + the shared sampler, which is what every single-texture
    /// reader here binds: each pass of the bloom chain, and the glow target —
    /// taken at group 0 by the composite that lays the light down and at group
    /// 1 by the node, marker and label pipelines, whose washes read the same field.
    ///
    /// Glow and bloom share linear filtering with ClampToEdge. Every glow
    /// reader uses normalized coordinates to reconstruct the reduced-resolution
    /// field, so changing this sampler also changes the washes and their edges.
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
    shadow_cell_pipelines: shadow::CellPipelines,
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
    target_format: wgpu::TextureFormat,
}

/// Mutable rendering state owned by one egui-wgpu callback context. Each new
/// context starts empty even when its pane IDs match a previous window.
struct LatticeResources {
    compiled: CompiledLatticeResources,
    /// This renderer's bindings for the sheets a glyph can be cut from — egui's
    /// shared font texture and the drawn marks' private texture.
    sheets: text::Sheets,
    panes: HashMap<u64, PaneBuffers>,
    /// GPU-side timing of the lattice passes. `None` when the device didn't
    /// grant timestamp queries — plenty of GPUs (and the offline renderer,
    /// which never asks for the feature) don't, and the readout says so
    /// rather than pretending.
    timer: Option<GpuTimer>,
    /// Last frame's shadow packing, filled again by the next pane to prepare.
    shadow_buffers: lattice_prepare::ShadowBuffers,
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
                let compiled = CompiledLatticeResources::new(device, format);
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

struct PaneBuffers {
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    instance_count: u32,
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
    /// The words of `shadow::node_occluders`, bound beside the casters.
    occluder_buffer: wgpu::Buffer,
    occluder_capacity: usize,
    caster_bind_group: wgpu::BindGroup,
    /// The scene pass's whole order (see [`Draw`]), held to what actually
    /// reached the buffers above.
    draws: Vec<Draw>,
    /// What the glyph shader is told about this pane: its size in points, the
    /// atlas's, and the terms a name's shadow is cast on.
    glyph_uniform_buffer: wgpu::Buffer,
    /// Names both sampled sheets, so it is rebuilt whenever either allocation
    /// is replaced — and `glyph_sheet_keys` is which bindings it names.
    glyph_bind_group: Option<wgpu::BindGroup>,
    glyph_sheet_keys: (u64, u64),
    /// GPU colour history, keyed on this pane's identity and row capacity,
    /// independently of the viewport targets. A release can have no current
    /// ink, so resizing must retain these rows rather than reseeding them.
    ink_history: Option<InkStrip>,
    ink_kernel: lattice_node_glow::InkKernel,
    material_tile: Option<lattice_material::Tile>,
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

/// Request the adapter's limits in every rendering shell. The lattice's three
/// RGBA16F attachments plus R16F local transmission require 26 bytes/sample;
/// supported Apple silicon and the measured CI Metal adapter provide at least 64.
/// Older adapters are unsupported; there is no second shadow pass to fall back to.
pub fn device_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let limits = adapter.limits();
    assert!(
        limits.max_color_attachment_bytes_per_sample >= 26,
        "the lattice requires at least 26 color attachment bytes per sample"
    );
    limits
}

impl CompiledLatticeResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        Self::new_with_progress(device, target_format, |_| {})
    }

    fn new_with_progress(
        device: &wgpu::Device,
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
        let glow_statistics_layout = lattice_node_glow::statistics_layout(device);
        let glyph_layout = text::glyph_bind_group_layout(device);
        let glyph_shader = text::glyph_shader(device, &text_source());
        let shaders = ShaderPipelines::new(
            device,
            &lattice_shader,
            &blit_shader,
            &glyph_shader,
            ShaderLayouts {
                scene: SceneLayouts {
                    uniforms: &bind_group_layout,
                    glow: &filter_layout,
                    shadow: &shadow_layout,
                    casters: &caster_layout,
                },
                glyph: &glyph_layout,
                strip: &strip_layout,
                statistics: &glow_statistics_layout,
            },
            &progress,
        );

        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_composite_bind_group_layout"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                texture_entry(2),
                uniform_entry(3),
                texture_entry(5),
                texture_entry(6),
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
        let bright_pipeline = filter("fs_bright");
        let downsample_pipeline = filter("fs_downsample");
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

        progress(startup::Stage::Shadows);
        let shadow_cell_pipelines = shadow::create_cell_pipelines(device, &shadow_layout);
        progress(startup::Stage::Interface);

        // The stand-in light: one transparent texel. It is the format the real
        // target is in so that one bind group layout serves both. The shared
        // linear sampler uses ClampToEdge, keeping every normalized-coordinate
        // read on this single transparent texel wherever the fragment stands.
        //
        // RENDER_ATTACHMENT alongside the binding though nothing ever draws
        // into it: that usage is what gives wgpu a way to zero-initialize the
        // texture, and a zero it cannot write is a texel of whatever the
        // driver left there smeared under every node's ink.
        let stand_in = |label, format| {
            crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
            )
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
            shaders,
            composite_pipeline,
            bright_pipeline,
            downsample_pipeline,
            blur_h_pipeline,
            blur_v_pipeline,
            glow_statistics_layout,
            material: lattice_material::Pipelines::new(device, &filter_layout),
            bind_group_layout,
            composite_layout,
            filter_layout,
            glow_dummy_bind_group,
            bloom_dummy: glow_dummy,
            strip_layout,
            sampler,
            shadow_cell_pipelines,
            shadow_dummy_bind_group,
            shadow_layout,
            caster_layout,
            glyph_layout,
            glyph_sampler: text::glyph_sampler(device),
            target_format,
        }
    }
}

impl LatticeResources {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, target_format: wgpu::TextureFormat) -> Self {
        Self::from_compiled(CompiledLatticeResources::new(device, target_format), device, queue)
    }

    fn from_compiled(
        compiled: CompiledLatticeResources,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Self {
        Self {
            compiled,
            sheets: text::Sheets::new(device, queue),
            panes: HashMap::new(),
            timer: GpuTimer::new(device, queue),
            shadow_buffers: Default::default(),
            #[cfg(feature = "hot-reload")]
            watcher: ShaderWatcher::new(),
        }
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
        let layout = &self.compiled.bind_group_layout;
        let caster_layout = &self.compiled.caster_layout;
        // Taken before the pane is borrowed: the view is a fresh handle onto
        // this frame's font texture and mark sheet — `prepare` binds them
        // before it gets here.
        let (glyph_layout, glyph_sampler) =
            (&self.compiled.glyph_layout, &self.compiled.glyph_sampler);
        let sheet_keys = self.sheets.keys();
        // Each of these mints a `wgpu::TextureView`, and the only thing that
        // reads them is the bind-group rebuild below — which the ordinary
        // frame skips, because the sheets it names have not moved. So they are
        // built off that rebuild's own predicate rather than beside the rest
        // of the frame's setup, and holding one IS the decision to rebuild.
        // #907 did this for the text callback and left this copy, which is the
        // same views in the same shape.
        //
        // The font atlas decides on its own account: the first frames of a
        // session arrive before one exists, and a pane cannot be bound against
        // a sheet that has not come.
        let wants_views = !self.sheets.atlas.is_empty()
            && self
                .panes
                .get(&pane_id)
                .is_none_or(|p| p.glyph_bind_group.is_none() || p.glyph_sheet_keys != sheet_keys);
        let views = wants_views.then(|| self.sheets.views());
        let shared = OffscreenShared {
            format: LATTICE_COLOR_FORMAT,
            composite_layout: &self.compiled.composite_layout,
            filter_layout: &self.compiled.filter_layout,
            shadow_layout: &self.compiled.shadow_layout,
            sampler: &self.compiled.sampler,
            bloom_dummy: &self.compiled.bloom_dummy,
            glow_statistics_layout: &self.compiled.glow_statistics_layout,
        };
        let want_casters = wants.casters;
        let want_occluders = wants.occluders;
        let pane = self.panes.entry(pane_id).or_insert_with(|| {
            let uniform_buffer = crate::gpu_memory::buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some("lattice_uniforms"),
                    size: std::mem::size_of::<Uniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lattice_bind_group"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });
            let (caster_buffer, occluder_buffer, caster_bind_group) = shadow::caster_buffers(
                device,
                caster_layout,
                INITIAL_BOX_CAPACITY,
                INITIAL_OCCLUDER_CAPACITY,
            );
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
                occluder_buffer,
                occluder_capacity: INITIAL_OCCLUDER_CAPACITY,
                caster_bind_group,
                draws: Vec::new(),
                glyph_uniform_buffer: crate::gpu_memory::buffer(
                    device,
                    &wgpu::BufferDescriptor {
                        label: Some("lattice_glyph_uniforms"),
                        size: std::mem::size_of::<text::TextUniforms>() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    },
                ),
                glyph_bind_group: None,
                glyph_sheet_keys: (u64::MAX, u64::MAX),
                ink_history: None,
                ink_kernel: Default::default(),
                material_tile: None,
                offscreen: None,
            }
        });
        // A bind group names one texture per sheet, and a sheet that GREW is a
        // new one — so a pane that prepared against the old texture has to be
        // handed the new one before it draws again. Rebuilt on the keys rather
        // than on the sizes, which is the cheap way to be right about the
        // same-size re-upload too: nothing is stale there, and one bind group
        // per publication is nothing.
        //
        // Holding views is exactly that condition, taken before the entry
        // above: a pane that entry has just created was absent when the
        // question was asked, and an absent pane reads as unbound.
        if let Some((atlas_view, mark_view)) = views {
            pane.glyph_bind_group = Some(text::bind_group(
                device,
                glyph_layout,
                glyph_sampler,
                &atlas_view,
                &mark_view,
                &pane.glyph_uniform_buffer,
            ));
            pane.glyph_sheet_keys = sheet_keys;
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
                offscreen.ensure_shadow(device, &shared, wants.shadow);
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
        // The casters' kernels and occluder grid, whose buffers and bind group
        // are one object: rebuilt together or the group names a buffer that is
        // gone.
        if want_casters > pane.caster_capacity || want_occluders > pane.occluder_capacity {
            pane.caster_capacity = pane.caster_capacity.max(want_casters.next_power_of_two());
            pane.occluder_capacity = pane.occluder_capacity.max(want_occluders.next_power_of_two());
            let (casters, occluders, bind_group) = shadow::caster_buffers(
                device,
                caster_layout,
                pane.caster_capacity,
                pane.occluder_capacity,
            );
            pane.caster_buffer = casters;
            pane.occluder_buffer = occluders;
            pane.caster_bind_group = bind_group;
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

/// And the words of the node occluder grid: a header, a few hundred offsets
/// and a handful of casters per bin.
const INITIAL_OCCLUDER_CAPACITY: usize = 4096;

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
    /// The ink strip's height: the row map's own capacity.
    rows: u32,
    /// How many casters this frame's kernels are packed for — the storage
    /// buffer at group 3 and the bind group naming it, which grow together
    /// (`shadow::caster_buffer`). Sized HERE rather than beside the vertex
    /// buffers below, because a storage buffer's bind group has to be rebuilt
    /// with it and this is where the layout is in scope.
    casters: usize,
    /// And how many words this frame's occluder grid is, bound in the same
    /// group (`shadow::caster_buffers`).
    occluders: usize,
}

/// A `capacity`-element vertex buffer (VERTEX | COPY_DST) sized for `T`.
/// Used for both the instance and marker buffers, which differ only in label
/// and element type.
fn create_vertex_buffer<T>(device: &wgpu::Device, label: &str, capacity: usize) -> wgpu::Buffer {
    crate::gpu_memory::buffer(
        device,
        &wgpu::BufferDescriptor {
            label: Some(label),
            size: (capacity * std::mem::size_of::<T>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        },
    )
}

/// Fill native staging directly; dropping the view schedules its copy. Byte
/// arrays require no typed alignment, and `write_iter` enforces the exact row
/// count. wgpu still allocates its staging buffer for each nonempty write.
/// `BYTES` is `T`'s size, which a generic cannot name as an array length.
fn write_rows<T: bytemuck::Pod, const BYTES: usize>(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    count: usize,
    rows: impl Iterator<Item = T>,
) {
    const { assert!(std::mem::size_of::<T>() == BYTES) };
    let Some(size) = wgpu::BufferSize::new((count * BYTES) as u64) else {
        assert_eq!(rows.count(), 0, "empty upload must have no rows");
        return;
    };
    let mut view = queue.write_buffer_with(buffer, 0, size).expect("valid row upload");
    let (chunks, tail) = view.slice(..).into_chunks::<BYTES>();
    debug_assert!(tail.is_empty());
    chunks.write_iter(rows.map(bytemuck::cast::<_, [u8; BYTES]>));
}
const BOX_BYTES: usize = std::mem::size_of::<shadow::ShadowBox>();
const INSTANCE_BYTES: usize = std::mem::size_of::<GpuInstance>();

mod gpu_harness;
// Dependent crates exercise the same GPU requirement as this crate's tests.
#[doc(hidden)]
pub use gpu_harness::{headless_device as test_gpu_device, test_gpu_adapter};
#[cfg(test)]
mod frame_timer;
#[cfg(test)]
mod lattice_tests;
