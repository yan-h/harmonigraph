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
//! while bloom is on the pass carries a second colour attachment without
//! their ink, and the bright pass reads that (see [`LatticeBloom::nodes_view`]).
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
pub use dot_shadow::dot_shadow_paint_callback;
pub use glow::{glow_paint_callback, GlowDot};

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
/// The bloom does NOT follow, though it would from a single-attachment pass:
/// see [`LatticeBloom::nodes_view`], which is the copy the bright pass reads.
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

/// The lattice's colour between its own passes: the node light, both scene
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
/// two-attachment form the offscreen pass draws through; the bare pair is
/// the single-attachment one the parity test's reference path uses; the
/// `glow` four are the glow's own pass — a billboard and the light it lays
/// down, once for the nodes and once for the resting markers; the `ink` four
/// are the strip the nodes' light is coloured out of, read and then blurred
/// ahead of it (see [`InkStrip`]); and the `cell` four rasterize a node's ink
/// and the Gaussian's one marker cross into the shadow atlas (`shadow.rs`).
#[cfg(any(test, feature = "hot-reload"))]
const LATTICE_ENTRY_POINTS: &[&str] = &[
    "vs_main",
    "fs_main",
    "fs_main_scene",
    "vs_plus",
    "fs_plus",
    "fs_plus_scene",
    "vs_glow",
    "fs_glow",
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
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x4, 2 => Float32x3, 3 => Uint32x3,
            4 => Float32, 6 => Uint32x2,
            7 => Float32x4, 8 => Float32x4, 10 => Float32, 11 => Float32,
            12 => Float32x4
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

        // Sort back-to-front along the view direction: alpha blending relies
        // on painter order, including every node, marker and label shadow.
        //
        // Sheets back to front FIRST, then painter's order within a sheet.
        // That is still just back-to-front — world z IS the sevens axis, and
        // the first key is only its depth — but it stays EXACT when the
        // camera is orbited, where two nodes on one sheet have different
        // depths and a plain depth sort interleaves the sheets. Interleaving
        // is not a cosmetic problem: it puts the markers in the wrong place in
        // the order, and every shadow in the frame is cast in that order — an
        // item multiplies what is already under it, so which of two overlapping
        // items darkens the other is exactly where each stands in this walk.
        //
        // Do not reorder the sheets on top of this. Forcing the home sheet
        // to the bottom (so off-sheet notes could never be hidden by it)
        // inverts the far half of the axis: the sheet BEHIND home then draws
        // last, over the home sheet in front of it. Grouping by distance from
        // home does the same thing more thoroughly. Depth is what the reader is
        // being shown; it is what the order has to follow.
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
        };

        let split = order.iter().position(|&(plane, _, _)| plane <= 0.0).unwrap_or(order.len());
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
        // Inside ONE home node the spacing is the cross standing at its
        // position, then the node's ink over it: a node covers the cross it
        // stands on, and shadows it, exactly as it covers the sheets behind.
        //
        // The markers are in this walk at all because a cross in FRONT of a
        // node covers that node. Face-on the case barely arises — a node's disc
        // reaches about its own cell, so the only cross near it is its own —
        // but tilt the sheet and one disc spans a dozen positions while the
        // billboard does not foreshorten with it, so where each cross stands in
        // the order is what the reader is being told about depth.
        // One node's ink as a box on the pane, in points — the space a caster
        // is packed in (`shadow::pack`). The billboard faces the camera's own
        // right and up, so those two axes project onto the screen's and one
        // corner along each bounds the circle the node fits inside.
        //
        // A node the projection cannot place — behind the eye, or collapsed to
        // nothing — casts no shadow rather than a box of infinities for the
        // packer to size.
        let to_points = |p: glam::Vec3| {
            let clip = view_proj * p.extend(1.0);
            (clip.w > 1e-4).then(|| {
                let ndc = clip / clip.w;
                [(ndc.x * 0.5 + 0.5) * size_points.x, (0.5 - ndc.y * 0.5) * size_points.y]
            })
        };
        let node_points = scene.node_radius * camera.points_per_world(size_points.y);
        // Each group's own σ in POINTS, read once: a caster carries it and the
        // packer needs no second conversion (`shadow::sigma_points`). A group
        // with either bar at the bottom hands over a σ of nothing, which is the
        // group's off switch all the way down — no cell, no atlas, no taps.
        let geometry = scene.shadow.lattice_geometry;
        let text = scene.shadow.lattice_text;
        let sigma_of = |style: harmonigraph_scene::ShadowStyle| {
            if style.casts() {
                shadow::sigma_points(style.width, node_points)
            } else {
                0.0
            }
        };
        let (geometry_sigma, text_sigma) = (sigma_of(geometry), sigma_of(text));
        // How far the GEOMETRY group's shadow reaches past its own ink, in
        // points — what a node's box is clipped to the pane by.
        let shadow_reach = geometry_sigma * geometry.kernel.reach_sigmas();
        let node_caster = |n: &harmonigraph_scene::NodeInstance, g: &GpuInstance| {
            // The circle the node's ink fits inside, in its own uv: `node_rim`
            // in lattice.wgsl, widened by the audio ring, which is dialled on
            // radii of its own and may stand outside the ring stack.
            let mut rim = scene.rings_outer.max(0.0);
            if (g.marks[0] | g.marks[1]) != 0 && scene.mark_thickness > 0.0 {
                rim = rim.max(scene.mark_inner + scene.mark_thickness);
            }
            if ringing && g.ring > 0.0 {
                rim = rim.max(scene.spectral.outer);
            }
            // uv 1 is 1.8 node radii of the node's own sheet (`node_vertex`),
            // which is the one conversion between the bars' unit and the world.
            let reach = rim * scene.node_radius * 1.8 * n.scale.max(0.05);
            let empty = shadow::Caster {
                rect: [0.0; 4],
                level: 0.0,
                sigma_points: geometry_sigma,
                kernel: geometry.kernel,
                direct_distance: false,
            };
            let (Some(c), Some(x), Some(y)) = (
                to_points(n.world_pos),
                to_points(n.world_pos + right * reach),
                to_points(n.world_pos + up * reach),
            ) else {
                return empty;
            };
            let half = |axis: usize| (x[axis] - c[axis]).abs().max((y[axis] - c[axis]).abs());
            let (hw, hh) = (half(0), half(1));
            if !(hw.is_finite() && hh.is_finite() && hw > 0.0 && hh > 0.0) {
                return empty;
            }
            // Clipped to the pane the shadow can land on, grown by the blur's
            // own reach. A perspective camera projects a node close to the eye
            // onto a box thousands of panes wide, and the packer sizes the
            // WHOLE atlas off the widest box it is handed: unclipped, one such
            // node takes the atlas to the device's limit and every cell packed
            // after it falls outside and casts nothing. Nothing is lost — a
            // caster's cell is a picture of what its shadow lands on, and past
            // this box it lands off the pane.
            let lo = glam::Vec2::splat(-shadow_reach);
            let hi =
                glam::Vec2::new(size_points.x, size_points.y) + glam::Vec2::splat(shadow_reach);
            let min = glam::Vec2::new(c[0] - hw, c[1] - hh).max(lo);
            let max = glam::Vec2::new(c[0] + hw, c[1] + hh).min(hi);
            if !(max.x > min.x && max.y > min.y) {
                return empty;
            }
            // LEVEL 1: the coverage the cell is filled with already carries
            // every layer's own envelope (`node_ink`), so a released node's
            // shadow fades with its ink and needs no second term here.
            shadow::Caster {
                rect: [min.x, min.y, max.x - min.x, max.y - min.y],
                level: 1.0,
                ..empty
            }
        };
        // One arm of a resting marker on this pane, in points. Every cross is
        // the same shape at the same size — they stand on the home sheet at one
        // radius (`derive_pluses`) — so one number answers for the whole field,
        // through the same conversion a node's own radius takes.
        let points_per_world = camera.points_per_world(size_points.y);
        let marker_arm_points =
            scene.pluses.first().map_or(0.0, |p| (p.radius * points_per_world).max(0.0));

        let mut instances = Vec::with_capacity(order.len());
        let mut pluses = Vec::with_capacity(scene.pluses.len());
        let mut glyphs = Vec::with_capacity(labels.glyphs.len());
        let mut casters: Vec<shadow::Caster> = Vec::new();
        let mut node_cells: Vec<u32> = Vec::with_capacity(order.len());
        // The marker field's caster, ahead of everything the walk pushes. Its
        // style is the TEXT group's, which is what a marker turns into and out
        // of as a name appears. A Gaussian gives it one shared cell centred on
        // a crossing; a distance keeps only the profile metadata and evaluates
        // the exact field in `plus_paint`.
        if marker_arm_points > 0.0 {
            let a = marker_arm_points;
            casters.push(shadow::Caster {
                rect: [-a, -a, 2.0 * a, 2.0 * a],
                level: 1.0,
                sigma_points: text_sigma,
                kernel: text.kernel,
                direct_distance: true,
            });
        }
        let mut draws: Vec<Draw> = Vec::with_capacity(order.len());
        let mut loose_drawn = false;
        for (k, &(_, _, i)) in order.iter().enumerate() {
            if k == split {
                push_loose(&mut draws, &mut pluses, &loose);
                loose_drawn = true;
            }
            let instance = to_gpu(&scene.nodes[i]);
            let ships = paints(&instance);
            // The cross, whether or not the node it stands on draws anything:
            // an idle position is exactly where a marker does its work, and the
            // node it belongs to is still what says how far off it is.
            if plus_of[i] != u32::MAX {
                push_plus(&mut draws, pluses.len() as u32);
                pluses.push(to_plus(&scene.pluses[plus_of[i] as usize]));
            }
            if ships {
                push_node(&mut draws, instances.len() as u32);
                // Its own cell of the atlas, beside it: what this node's shadow
                // is a blur of, and what it multiplies the frame under it by.
                node_cells.push(casters.len() as u32);
                casters.push(node_caster(&scene.nodes[i], &instance));
                instances.push(instance);
            }
            // The name, immediately after the node it names — so what covers a
            // name is exactly what covers its node.
            let (start, count) = glyphs_of[i];
            if count > 0 {
                let at = glyphs.len() as u32;
                let run = &labels.glyphs[start as usize..(start + count) as usize];
                glyphs.extend_from_slice(run);
                draws.push(Draw::Label(at, at + count, casters.len() as u32));
                casters.push(shadow::caster_of(run, text_sigma, text.kernel));
            }
        }
        // The home run can be empty and can run to the end of the order, in
        // which case the walk never reached `split`.
        if !loose_drawn {
            push_loose(&mut draws, &mut pluses, &loose);
        }

        LatticeCallback {
            instances,
            glyphs,
            casters,
            node_cells,
            shadow: scene.shadow,
            marker_arm_points,
            draws,
            atlas: labels.atlas,
            marks: labels.marks,
            sdf: labels.sdf,
            slide: labels.slide,
            pluses,
            uniforms: Uniforms {
                composite: CompositeParams {
                    darkest_pitch: scene.darkest_pitch,
                    brightest_pitch: scene.brightest_pitch,
                    render_scale,
                    bloom_strength: bloom_strength(scene.bloom_strength),
                },
                camera: CameraParams {
                    view_proj: Matrix4(view_proj.to_cols_array_2d().map(Float4)),
                    right: Float4(right.extend(0.0).to_array()),
                    up: Float4(up.extend(0.0).to_array()),
                },
                node: NodeParams {
                    radius: scene.node_radius,
                    band_inner: scene.outer_inner,
                    band_outer: scene.outer_outer,
                    rings_outer: scene.rings_outer,
                    mark_inner: scene.mark_inner,
                    angular_gap: scene.octave_gap,
                    mark_thickness: scene.mark_thickness,
                    padding: 0.0,
                },
                marker: MarkerParams {
                    half_width: scene.plus_half_width,
                    taper_start: scene.plus_taper_start,
                    world_unit: scene.marker_unit,
                    padding: 0.0,
                },
                octave: OctaveParams {
                    span: scene.octave_layout.span as f32,
                    center: scene.octave_layout.center,
                    padding: Float2([0.0; 2]),
                    bounds: std::array::from_fn(|row| {
                        Float4(std::array::from_fn(|col| scene.octave_layout.bounds[row * 4 + col]))
                    }),
                },
                shimmer: ShimmerParams {
                    slide: scene.shimmer_slide(),
                    period: scene.shimmer_width,
                    intensity: scene.shimmer_intensity,
                    softness: scene.shimmer_softness,
                    pattern: scene.pulse_marks.shader_index() as f32,
                    padding0: 0.0,
                    padding1: 0.0,
                    padding2: 0.0,
                },
                spectral: SpectralParams {
                    inner: scene.spectral.inner,
                    outer: scene.spectral.outer,
                    range_cents: scene.spectral.range,
                    folded: f32::from(u8::from(scene.spectral.folded)),
                },
                glow: if lights {
                    GlowParams {
                        reach: scene.glow_reach,
                        strength: scene.glow_strength,
                        blend: scene.glow_blend,
                        curve: scene.glow_curve.shape(),
                        wash: scene.glow_wash,
                        row_capacity: scene.glow_rows.max(1) as f32,
                        padding: Float2([0.0; 2]),
                    }
                } else {
                    bytemuck::Zeroable::zeroed()
                },
                // Every shadow still casts with the glow disabled. Markers
                // inherit notation's style even though this pipeline draws them.
                geometry_shadow: ShadowParams {
                    width: geometry.width,
                    reach_sigmas: geometry.kernel.reach_sigmas(),
                    depth: geometry.depth,
                    padding: 0.0,
                },
                marker_shadow: ShadowParams {
                    width: text.width,
                    reach_sigmas: text.kernel.reach_sigmas(),
                    depth: text.depth,
                    padding: 0.0,
                },
                shadow_target: ShadowTargetParams {
                    pane_points: Float2([size_points.x, size_points.y]),
                    atlas_texels: Float2([0.0; 2]),
                },
                // Settled by prepare after packing casters[0].
                marker_cell: bytemuck::Zeroable::zeroed(),
                lattice_ground: Float4(scene.lattice_ground.to_array()),
                pitch_lut: std::array::from_fn(|k| Float4(scene.pitch_lut[k].to_array())),
                spectral_lut: std::array::from_fn(|k| Float4(scene.spectral.lut[k].to_array())),
                // No shader reader while the ring is off; skip the bucket pack.
                spectrum_color: if scene.spectral.ring_draws() {
                    pack_spectrum(&scene.spectral.color_levels).map(Uint4)
                } else {
                    [Uint4([0; 4]); SPECTRUM_WORDS]
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
    /// or composited: no target, no pass, and every wash reading the stand-in
    /// transparent texture rather than a light. The SHADOW is not gated by it
    /// — an item casts with no light in the picture (see
    /// [`ShadowParams`]).
    fn glow_draws(&self) -> bool {
        self.uniforms.glow.reach > 0.0
    }
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct LatticeResources {
    scenes: [ScenePipelines; 2],
    composite_pipeline: wgpu::RenderPipeline,
    /// Bloom chain: bright pass, half->quarter downsample, blur x2.
    bright_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    blur_h_pipeline: wgpu::RenderPipeline,
    blur_v_pipeline: wgpu::RenderPipeline,
    /// The node glow's own pass: one draw over the node instance buffer, into
    /// a target of the glow's own (see [`create_glow_pipeline`]).
    glow_pipeline: wgpu::RenderPipeline,
    /// The colour it draws in, settled ahead of it: the ink read round every
    /// node, then blurred (see [`create_ink_strip_pipelines`]).
    ink_strip_pipeline: wgpu::RenderPipeline,
    ink_blur_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
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
    /// This renderer's bindings for the two sheets a glyph can be cut from —
    /// egui's shared font texture and the drawn marks' private texture.
    atlas: text::AtlasTexture,
    marks: text::AtlasTexture,
    blank: wgpu::Texture,
    /// Identity of the shared SDF texture its glyph bind groups name. The
    /// allocation itself is stored once in `CallbackResources` and is also
    /// used by the standalone text renderer.
    sdf_key: u64,
    blank_sdf: wgpu::Texture,
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
    offscreen: Option<Offscreen>,
}

/// The per-pane offscreen render target and bloom chain, recreated when
/// the pane's pixel size (or render scale) changes.
///
/// The scene target uses the render-scaled size; the bloom textures use
/// fractions of the pane's NATIVE screen size, so the halo's on-screen
/// width doesn't change with the render-scale setting.
struct Offscreen {
    /// The descriptor format shared by both scene attachments.
    #[cfg(test)]
    format: wgpu::TextureFormat,
    color_view: wgpu::TextureView,
    /// The independent label-free scene attachment and its filtered halo.
    /// Present only while bloom is on; toggling it never replaces glow history.
    bloom: Option<LatticeBloom>,
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

/// The allocations whose contents are needed only while bloom is enabled.
struct LatticeBloom {
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
    /// size — about 28 MB for a Retina-sized pane at scale 1 now that the
    /// lattice works in half floats, and it grows with the square of the render
    /// scale like the main scene target. There is one more colour write per
    /// node fragment and nothing else — no extra pass, no extra draw call, no
    /// extra geometry.
    ///
    /// There is also no cheaper slot. The bright pass samples a finished
    /// texture, so "after bloom but still interleaved with the nodes" does not
    /// exist, and anything short of a second attachment (a stencil, a
    /// threshold) buys back the memory by punching a hole in the node's own
    /// halo where the name sits — which is the artifact this removes.
    nodes_view: wgpu::TextureView,
    chain: BloomChain,
}

/// Where a frame's node light is assembled before any of it reaches the
/// picture: one transparent premultiplied colour texture at the scene's own
/// size, plus the bind group its readers take it through.
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
/// each other, and a target left allocated at reach 0 is a scene-sized texture
/// held for a feature that is off.
struct GlowTarget {
    /// The descriptor format of `view`.
    #[cfg(test)]
    format: wgpu::TextureFormat,
    view: wgpu::TextureView,
    /// The texture + the shared sampler, as
    /// [`LatticeResources::filter_layout`] takes them.
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
/// spends about 400 KB on the pair — a rounding error beside the pane-sized
/// half-float attachments. That is what the light costs in memory to stop
/// costing a whole reading of the node per lit fragment.
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
/// hands the light. The format is explicit rather than inherited from the
/// scene because a strip texel is a normalised colour beside a WEIGHT, and a
/// weight is a layer's level times its width — small numbers that must not be
/// quantized away if the scene format changes.
const INK_STRIP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The shared, pane-independent objects an [`Offscreen`] binds against.
struct OffscreenShared<'a> {
    format: wgpu::TextureFormat,
    composite_layout: &'a wgpu::BindGroupLayout,
    /// One texture plus the sampler; see [`LatticeResources::filter_layout`].
    filter_layout: &'a wgpu::BindGroupLayout,
    /// One texture, unfiltered: what both stages of the ink strip are read
    /// through (see [`InkStrip`]).
    strip_layout: &'a wgpu::BindGroupLayout,
    /// The shadow atlas as its readers take it; see
    /// [`LatticeResources::shadow_layout`].
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
        carried_strip: Option<InkStrip>,
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
        let composite_bind_group = Self::composite_binding(
            device,
            shared,
            uniform_buffer,
            &color_view,
            shared.bloom_dummy,
        );

        Offscreen {
            #[cfg(test)]
            format,
            bloom: None,
            glow: None,
            shadow: None,
            carried_strip,
            composite_bind_group,
            color_view,
            size,
            screen_size,
        }
    }

    fn composite_binding(
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        uniforms: &wgpu::Buffer,
        color: &wgpu::TextureView,
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
            let nodes_view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("lattice_offscreen_nodes"),
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
                .create_view(&Default::default());
            let chain = BloomChain::new(
                device,
                "lattice",
                shared.format,
                shared.filter_layout,
                shared.sampler,
                &nodes_view,
                self.screen_size,
            );
            LatticeBloom { nodes_view, chain }
        });
        self.composite_bind_group = Self::composite_binding(
            device,
            shared,
            uniforms,
            &self.color_view,
            self.bloom.as_ref().map_or(shared.bloom_dummy, |b| &b.chain.quarter_a_view),
        );
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
    fn new(device: &wgpu::Device, shared: &OffscreenShared<'_>, size: [u32; 2], rows: u32) -> Self {
        let OffscreenShared { format, filter_layout, strip_layout, sampler, .. } = *shared;
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

/// The three bind group layouts a scene pipeline draws through: the pane's
/// uniforms at group 0, the finished light at group 1 — the field a node washes
/// its own ink with (`node_paint`) — and the shadow atlas at group 2, the cell
/// each draw multiplies the frame by.
///
/// Both the node and the marker pipeline take all three: they are one pass over
/// one pane, so one layout is what lets the light and the atlas be bound once
/// for both. Whether there IS either to bind is the caller's business — see
/// `LatticeResources::glow_dummy_bind_group` and `shadow_dummy_bind_group`.
#[derive(Clone, Copy)]
struct SceneLayouts<'a> {
    uniforms: &'a wgpu::BindGroupLayout,
    glow: &'a wgpu::BindGroupLayout,
    shadow: &'a wgpu::BindGroupLayout,
    /// Every caster's kernel; see [`LatticeResources::caster_layout`].
    casters: &'a wgpu::BindGroupLayout,
}

/// The lattice's own shader module, out of a WHOLE module's source: what
/// [`with_common`] builds at startup, or what the watcher reads back off disk
/// on a reload. Never lattice.wgsl alone, which names what common.wgsl
/// declares.
///
/// The four pipelines cut from that text each build their own module of it, so
/// this is the one place the text becomes a module and the one place that
/// contract is stated.
fn lattice_module(device: &wgpu::Device, shader_src: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("lattice_shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    })
}

/// Build one of the scene pipelines from WGSL source (startup uses the
/// baked-in source; hot-reload rebuilds from disk). Node and marker pipelines
/// share the module, bind group layout, blending, and topology; only entry
/// points and vertex layout differ.
///
/// `bloom` selects the second colour attachment, the independent input the
/// bright pass reads (see [`LatticeBloom::nodes_view`]). The single-attachment
/// variant serves production with bloom off and the direct parity reference.
/// Both rely on painter order and carry no depth state.
fn create_pipeline(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    layouts: SceneLayouts<'_>,
    entry_points: (&str, &str),
    vertex_layouts: &[wgpu::VertexBufferLayout<'_>],
    bloom: bool,
) -> wgpu::RenderPipeline {
    let shader = lattice_module(device, shader_src);

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

    // The offscreen pass's second attachment takes the same fragment under the
    // same blending: it is the same picture, drawn again without the labels.
    let color_target = wgpu::ColorTargetState {
        format: target_format,
        // Shader outputs premultiplied alpha.
        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    };
    let targets: &[Option<wgpu::ColorTargetState>] = if bloom {
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
            buffers: vertex_layouts,
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
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Build both scene pipelines from one source. `bloom` picks the
/// two-attachment fragment entry points along with the pass state that goes
/// with them — the pair travels together, since a pipeline whose shader
/// writes one attachment cannot be used in a pass that carries two.
fn create_pipelines(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    layouts: SceneLayouts<'_>,
    bloom: bool,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let (node, plus) =
        if bloom { ("fs_main_scene", "fs_plus_scene") } else { ("fs_main", "fs_plus") };
    (
        create_pipeline(
            device,
            shader_src,
            target_format,
            layouts,
            ("vs_main", node),
            &[GpuInstance::LAYOUT, shadow::ShadowBox::BESIDE_NODES],
            bloom,
        ),
        create_pipeline(
            device,
            shader_src,
            target_format,
            layouts,
            ("vs_plus", plus),
            &[GpuPlus::LAYOUT],
            bloom,
        ),
    )
}

/// The ordered scene pass's attachment-compatible draws. Index 0 carries
/// only the picture; index 1 also writes the independent bloom input.
/// Startup and hot reload build both through the same factory.
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
    source: &str,
    glyph_shader: &wgpu::ShaderModule,
    layouts: SceneLayouts<'_>,
    glyph_layout: &wgpu::BindGroupLayout,
) -> [ScenePipelines; 2] {
    [false, true].map(|bloom| {
        let (nodes, pluses) =
            create_pipelines(device, source, LATTICE_COLOR_FORMAT, layouts, bloom);
        ScenePipelines {
            nodes,
            pluses,
            glyph_fill: text::create_text_pipeline(
                device,
                glyph_shader,
                LATTICE_COLOR_FORMAT,
                glyph_layout,
                Some(layouts.glow),
                ("vs_glyph", "fs_fill_lit"),
                bloom,
                EGUI_BLEND,
            ),
            shadow_box: text::create_shadow_box_pipeline(
                device,
                glyph_shader,
                glyph_layout,
                layouts.shadow,
                layouts.casters,
                LATTICE_COLOR_FORMAT,
                bloom,
            ),
            glow_over: create_glow_over_pipeline(device, LATTICE_COLOR_FORMAT, layouts.glow, bloom),
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
    shader_src: &str,
    uniforms: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    const MAX_COMPONENT: wgpu::BlendComponent = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Max,
    };
    let shader = lattice_module(device, shader_src);
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
                module: &shader,
                entry_point: Some(entries.0),
                compilation_options: Default::default(),
                buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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

/// The glow pass's one attachment and the blend the light melds under.
///
/// **SCREEN**: `src + dst * (1 - src)`, premultiplied on both channels, is what
/// makes two neighbouring nodes' halos MELD: an overlap is brighter than either
/// alone, it is bounded by white however many nodes reach the same pixel, and
/// the operation is commutative, so nothing about the order inside a draw is
/// readable in the picture. Adding instead blows a chord's middle out to white
/// and makes the count of overlapping nodes, rather than any note, the
/// brightest thing on screen.
///
/// A MAX blend is the same guarantee taken further — an overlap exactly as
/// bright as the brighter node — and is not what the light wants: it is a
/// `max` of two smooth fields, so it creases along every locus where two nodes
/// light a pixel equally, and the lattice comes out gridded with the seams of
/// its own Voronoi diagram.
fn glow_targets(target_format: wgpu::TextureFormat) -> [Option<wgpu::ColorTargetState>; 1] {
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
    [Some(wgpu::ColorTargetState {
        format: target_format,
        blend: Some(screen),
        write_mask: wgpu::ColorWrites::ALL,
    })]
}

/// The node glow's one pipeline: the light, over the node instance buffer and
/// into the glow's own target (see [`GlowTarget`]).
///
/// **One attachment**, screen-blended ([`glow_targets`], which holds the
/// argument for it). Commutative and never subtractive, so no draw order is
/// readable in the light and the sheets meld in whatever order they are
/// handed.
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
fn create_glow_pipeline(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    strip_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = lattice_module(device, shader_src);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_glow_pipeline_layout"),
        bind_group_layouts: &[Some(bind_group_layout), Some(strip_layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fs_glow"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_glow"),
            compilation_options: Default::default(),
            buffers: &[GpuInstance::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_glow"),
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
    let shader = lattice_module(device, shader_src);
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
/// With bloom on it writes both the picture and the independent bloom input.
/// With bloom off the existing single-attachment blit samples the same light.
/// Both use painter order and the same premultiplied blend.
fn create_glow_over_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    light_layout: &wgpu::BindGroupLayout,
    bloom: bool,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit_shader"),
        source: wgpu::ShaderSource::Wgsl(BLIT_SRC.into()),
    });
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
    let targets = [Some(target.clone()), Some(target)];
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
            entry_point: Some(if bloom { "fs_glow_over" } else { "fs_blit" }),
            compilation_options: Default::default(),
            targets: &targets[..if bloom { 2 } else { 1 }],
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
        // Ahead of the scene pipelines because they take both at groups 1 and
        // 2: a node washes its own ink with the light the pass has just
        // composited, and multiplies the frame under it by its own cell of the
        // atlas.
        let shadow_layout = shadow::read_layout(device);
        let caster_layout = shadow::caster_layout(device);
        // The whole module, common half and all, built once for the four
        // pipelines cut from it.
        let shader_src = with_common(SHADER_SRC);
        let (node_cell_pipeline, plus_cell_pipeline) =
            create_cell_pipelines(device, &shader_src, &bind_group_layout);
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
        let glow_pipeline = create_glow_pipeline(
            device,
            &shader_src,
            LATTICE_COLOR_FORMAT,
            &bind_group_layout,
            &strip_layout,
        );
        let (ink_strip_pipeline, ink_blur_pipeline) =
            create_ink_strip_pipelines(device, &shader_src, &bind_group_layout, &strip_layout);

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
        let filter =
            |entry| create_post_pipeline(device, entry, LATTICE_COLOR_FORMAT, &filter_layout, None);
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

        // The label pipelines share both attachment choices with the scene,
        // preserving each label's place in painter order.
        let glyph_layout = text::glyph_bind_group_layout(device);
        // Compiled once for the three pipelines below, as `shader_src` is for
        // the lattice's.
        let glyph_shader = text::glyph_shader(device, &text_source());
        let scenes = create_scene_pipelines(
            device,
            &shader_src,
            &glyph_shader,
            SceneLayouts {
                uniforms: &bind_group_layout,
                glow: &filter_layout,
                shadow: &shadow_layout,
                casters: &caster_layout,
            },
            &glyph_layout,
        );
        let (
            glyph_coverage_cell_pipeline,
            glyph_distance_cell_pipeline,
            glyph_distance_pad_pipeline,
        ) = text::create_glyph_cell_pipelines(device, &glyph_shader, &glyph_layout);
        let shadow_cell_pipelines = shadow::create_cell_pipelines(device, &shadow_layout);

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

        LatticeResources {
            scenes,
            composite_pipeline,
            bright_pipeline,
            downsample_pipeline,
            blur_h_pipeline,
            blur_v_pipeline,
            glow_pipeline,
            ink_strip_pipeline,
            ink_blur_pipeline,
            bind_group_layout,
            composite_layout,
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
            atlas: text::AtlasTexture::default(),
            marks: text::AtlasTexture::default(),
            blank: text::blank_atlas(device, queue),
            sdf_key: 0,
            blank_sdf: text::blank_sdf_atlas(device, queue),
            target_format,
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
        let layout = &self.bind_group_layout;
        let caster_layout = &self.caster_layout;
        // Taken before the pane is borrowed: the view is a fresh handle onto
        // this frame's font texture and mark sheet — `prepare` binds them
        // before it gets here.
        let (glyph_layout, glyph_sampler) = (&self.glyph_layout, &self.glyph_sampler);
        let atlas_view = self.atlas.view();
        let mark_view = self.marks.view_or(&self.blank);
        let sdf_view = sdf.unwrap_or(&self.blank_sdf).create_view(&Default::default());
        let sheet_keys = (self.atlas.key(), self.marks.key(), self.sdf_key);
        let shared = OffscreenShared {
            format: LATTICE_COLOR_FORMAT,
            composite_layout: &self.composite_layout,
            filter_layout: &self.filter_layout,
            strip_layout: &self.strip_layout,
            shadow_layout: &self.shadow_layout,
            sampler: &self.sampler,
            bloom_dummy: &self.bloom_dummy,
        };
        let want_casters = wants.casters;
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
                caster_bind_group,
                draws: Vec::new(),
                glyph_uniform_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("lattice_glyph_uniforms"),
                    size: std::mem::size_of::<text::TextUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                glyph_bind_group: None,
                glyph_sheet_keys: (u64::MAX, u64::MAX, u64::MAX),
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
        // The shell publishes egui's current font texture under its concrete
        // wgpu type before callbacks prepare. Clone the handle before taking a
        // mutable borrow from the resource map; its pixels remain shared.
        let shared_atlas = callback_resources.get::<wgpu::Texture>().cloned();
        let shared_sdf =
            text::shared_sdf_texture(device, queue, callback_resources, self.sdf.as_ref());
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
        // rejected with a message; the pipelines in hand keep rendering.
        #[cfg(feature = "hot-reload")]
        if let Some(reloaded) = resources.watcher.poll() {
            // Both modules or neither. They are compiled against one
            // common.wgsl, so an edit in there is due to both, and committing
            // whichever half happened to compile would leave a name's shadow on
            // one build of `shadow_transmittance` and a node's on another —
            // which is the split this reload exists to close, not to make.
            let checked = validate_wgsl(
                "lattice.wgsl",
                &reloaded.lattice,
                reloaded.seam,
                LATTICE_ENTRY_POINTS,
            )
            .and_then(|()| {
                validate_wgsl("text.wgsl", &reloaded.text, reloaded.seam, text::TEXT_ENTRY_POINTS)
            })
            .and_then(|()| {
                validate_wgsl(
                    "roll.wgsl",
                    &module_source(&reloaded.common, roll::ROLL_SRC),
                    reloaded.seam,
                    roll::ROLL_ENTRY_POINTS,
                )
            })
            .and_then(|()| {
                validate_wgsl(
                    "dot_shadow.wgsl",
                    &module_source(&reloaded.common, dot_shadow::SRC),
                    reloaded.seam,
                    dot_shadow::ENTRY_POINTS,
                )
            });
            match checked {
                Ok(()) => {
                    let source = &reloaded.lattice;
                    // ...and the two draws that fill the atlas the pair above
                    // reads: a node's shadow is a blur of the same ink an edit
                    // just changed, so the cell has to be rasterized by the
                    // same build that draws the node.
                    let (node_cell_pipeline, plus_cell_pipeline) =
                        create_cell_pipelines(device, source, &resources.bind_group_layout);
                    // The glow off the same source, so an edit to a node's
                    // layers reaches the light around it in the same reload —
                    // they are one shader drawing one node, and reloading half
                    // of it is a halo of the previous build.
                    let glow_pipeline = create_glow_pipeline(
                        device,
                        source,
                        LATTICE_COLOR_FORMAT,
                        &resources.bind_group_layout,
                        &resources.strip_layout,
                    );
                    // ...and the strip the light is coloured out of, on the
                    // same argument one step further back: an edit to what a
                    // layer paints is an edit to what the halo is made of.
                    let (ink_strip_pipeline, ink_blur_pipeline) = create_ink_strip_pipelines(
                        device,
                        source,
                        &resources.bind_group_layout,
                        &resources.strip_layout,
                    );
                    resources.node_cell_pipeline = node_cell_pipeline;
                    resources.plus_cell_pipeline = plus_cell_pipeline;
                    resources.glow_pipeline = glow_pipeline;
                    resources.ink_strip_pipeline = ink_strip_pipeline;
                    resources.ink_blur_pipeline = ink_blur_pipeline;

                    // The NAMES, off the other module the same edit changed.
                    // All three of their pipelines: the fill is the ink
                    // standing in the light, the cell is the ink its shadow is
                    // blurred from, and the box is where that shadow is spent —
                    // one shader drawing one name, on the same argument the
                    // glow's rebuild above is made on.
                    let glyph_shader = text::glyph_shader(device, &reloaded.text);
                    resources.scenes = create_scene_pipelines(
                        device,
                        source,
                        &glyph_shader,
                        SceneLayouts {
                            uniforms: &resources.bind_group_layout,
                            glow: &resources.filter_layout,
                            shadow: &resources.shadow_layout,
                            casters: &resources.caster_layout,
                        },
                        &resources.glyph_layout,
                    );
                    let (
                        glyph_coverage_cell_pipeline,
                        glyph_distance_cell_pipeline,
                        glyph_distance_pad_pipeline,
                    ) = text::create_glyph_cell_pipelines(
                        device,
                        &glyph_shader,
                        &resources.glyph_layout,
                    );
                    resources.glyph_coverage_cell_pipeline = glyph_coverage_cell_pipeline;
                    resources.glyph_distance_cell_pipeline = glyph_distance_cell_pipeline;
                    resources.glyph_distance_pad_pipeline = glyph_distance_pad_pipeline;

                    // And the text CALLBACK's own glyph pipelines, in an entry
                    // of the map this one cannot reach: publishing raises the
                    // count they watch, and the next prepare that reads it
                    // rebuilds them against this same source.
                    reload::publish(reloaded.text, reloaded.common);
                    eprintln!("[harmonigraph-render] shaders hot-reloaded");
                }
                Err(err) => {
                    eprintln!("[harmonigraph-render] shader reload REJECTED, keeping old pipelines:\n{err}");
                }
            }
        }

        // The labels' sheets, before anything is encoded: the glyphs are drawn
        // inside the scene pass below, so the textures they read have to be the
        // current ones by the time that pass is recorded.
        let write_start = std::time::Instant::now();
        resources.bind_sheets(
            device,
            queue,
            shared_atlas.as_ref(),
            self.atlas.as_ref(),
            self.marks.as_ref(),
            shared_sdf.key,
        );
        // The first frames of a session can arrive before any pane has drawn a
        // glyph, and the labels wait for a font texture. Gated on the FONT
        // atlas alone: a mark is always drawn beside a letter, so a frame with
        // a mark in it is a frame with type in it.
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
        // Every caster's cell, packed for this frame (`shadow::pack`): the
        // Gaussian's one marker cross, one per node and one per name, each at
        // the resolution its own σ asks for. A caster whose group has either
        // bar at the bottom arrived with a σ of nothing and takes none, so a
        // frame with every group shut allocates no atlas and every cell reader
        // multiplies by exactly 1.
        //
        // The scale handed over is the TARGET's pixels per pane point — the
        // device's times the render scale, which is the term #496 found missing
        // from the field's reach.
        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let packed = shadow::pack(&self.casters, ppp * self.render_scale, max_dim);
        // A placeholder box preserves the caster index for a distance field
        // evaluated directly by its scene draw. Only a real cell asks for the
        // atlas; a markers-only Distance frame therefore allocates no atlas.
        let has_shadow_cells = packed.boxes.iter().any(|b| b.cell[2] > 0.0 && b.cell[3] > 0.0);
        let shadow_wanted = has_shadow_cells.then_some(packed.size);
        // The blur chain runs, and its intermediate is held, when some cell in
        // the atlas holds COVERAGE — which is what the chain convolves, a
        // distance cell collapsing its own quad inside both passes (`vs_cell`
        // in shadow.wgsl). Read off the boxes rather than off the settings, so
        // a Gaussian group with nothing in the frame to cast costs neither the
        // pass nor the plane.
        let blurs =
            packed.boxes.iter().any(|b| b.cell[2] > 0.0 && b.who[1] < 0.5 * shadow::DISTANCE_KIND);
        let pane = resources.pane_buffers(
            device,
            self.pane_id,
            offscreen_size,
            screen_size,
            PaneTargets {
                bloom: self.uniforms.composite.bloom_strength > 0.0,
                glow,
                shadow: shadow_wanted,
                blurs,
                // The strip's height is the row map's CAPACITY, which the
                // light's own clock hands out and which has nothing to do with
                // how many nodes this frame draws (`Scene::glow_rows`).
                rows: self.uniforms.glow.row_capacity as u32,
                // At least one: an empty storage binding is a validation error,
                // and a frame that packed nothing still binds the array for the
                // pipeline's layout — one zeroed entry, which is a caster with
                // no cells and a multiply of 1.
                casters: packed.casters.len().max(1),
            },
            shared_sdf.texture.as_ref(),
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

        if self.pluses.len() > pane.plus_capacity {
            pane.plus_capacity = self.pluses.len().next_power_of_two();
            pane.plus_buffer =
                create_vertex_buffer::<GpuPlus>(device, "lattice_pluses", pane.plus_capacity);
        }
        pane.plus_count = self.pluses.len() as u32;
        if !self.pluses.is_empty() {
            queue.write_buffer(&pane.plus_buffer, 0, bytemuck::cast_slice(&self.pluses));
        }

        // Every caster's whole kernel, for the SCENE draws: one entry a node, a
        // marker and a name all reach by the caster's own index. Written
        // whatever the packing says — a frame that packed nothing writes one
        // zeroed entry, which is a caster with no cells and a multiply of 1.
        let entries: &[shadow::ShadowCaster] =
            if packed.casters.is_empty() { &[shadow::NO_CASTER] } else { &packed.casters };
        queue.write_buffer(&pane.caster_buffer, 0, bytemuck::cast_slice(entries));
        pane.caster_count = packed.casters.len();

        // The casters' boxes, whether or not there is a font sheet to cut a
        // name out of: a node and a marker cast without one.
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
        }
        // Each node instance's own box beside it, for the two draws that bind
        // the pair (`shadow::ShadowBox::BESIDE_NODES`). Written whatever the
        // packing says: a frame that packed nothing hands every instance a box
        // of zeros, which is a caster with no cell and a multiply of 1.
        if pane.instance_count as usize > pane.node_cell_capacity {
            pane.node_cell_capacity = (pane.instance_count as usize).next_power_of_two();
            pane.node_cell_buffer = create_vertex_buffer::<shadow::ShadowBox>(
                device,
                "lattice_node_cells",
                pane.node_cell_capacity,
            );
        }
        if pane.instance_count > 0 {
            let all = &packed.boxes;
            let boxes: Vec<shadow::ShadowBox> = self
                .node_cells
                .iter()
                .map(|&i| all.get(i as usize).copied().unwrap_or(shadow::NO_CELL))
                .collect();
            debug_assert_eq!(boxes.len(), self.instances.len(), "one box per node instance");
            queue.write_buffer(&pane.node_cell_buffer, 0, bytemuck::cast_slice(&boxes));
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
            if !packed.boxes.is_empty() {
                // Each glyph's own name's box beside it, for the cell draw. The
                // runs are contiguous in draw order, so this is the boxes
                // repeated by their runs' lengths.
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
                        // The TEXT group's own depth, where `geometry_shadow` carries
                        // the geometry group's: a name's box is drawn by this
                        // pipeline and a ring's by the lattice's, so the two
                        // depths reach the two shaders through their own
                        // uniforms and never through each other.
                        //
                        // The group's WIDTH is not here: it is σ, and σ is
                        // spent on the CPU where the cells are packed.
                        shadow_depth: self.shadow.lattice_text.depth,
                        // The atlas the cells are drawn into, which may be
                        // larger than this frame's layout (`ensure_shadow`).
                        shadow_atlas_size: atlas_size,
                        // A lattice name paints no rim, so there is no ring for
                        // the two passes that draw one to walk. Zero samples is
                        // what says so (`ring`), and it is what keeps the fill's
                        // quad down to the reconstruction filter's own margin.
                        _pad: [0.0; 4],
                    }),
                );
            }
        } else {
            pane.glyph_count = 0;
        }

        // The order, held to what actually reached the buffers: every index in
        // a `Draw` addresses one of four lists and the pass draws off them
        // without a second look, so a draw naming something they do not hold is
        // dropped here rather than trusted there. The names go the same way
        // when there is no atlas — nothing to sample.
        pane.draws.clear();
        pane.draws.extend(self.draws.iter().copied().filter(|draw| match *draw {
            Draw::Nodes(a, b) => a < b && b <= pane.instance_count,
            Draw::Pluses(a, b) => a < b && b <= pane.plus_count,
            Draw::Label(a, b, l) => {
                a < b && b <= pane.glyph_count && (l as usize) < self.casters.len()
            }
        }));

        // What the shader is told about the atlas, settled HERE because the
        // atlas is sized here: its texels, for the two draws that fill a cell
        // and so cannot read the texture they are writing, and the markers'
        // shared blur cell, which is `casters[0]` wherever that row casts.
        //
        // The arm in points is the one term `pack` has no way to know, and it
        // is what maps a fragment's place on a cross into a cell no cross
        // placed (`vs_plus`).
        let mut uniforms = self.uniforms;
        if let Some(atlas) = pane.offscreen.as_ref().and_then(|o| o.shadow.as_ref()) {
            uniforms.shadow_target.atlas_texels = Float2(atlas.size.map(|v| v as f32));
        }
        if let Some(cell) = packed.boxes.first().filter(|_| self.marker_arm_points > 0.0) {
            uniforms.marker_cell = MarkerCellParams {
                rect: Float4(cell.rect),
                cell: Float4(cell.cell),
                points_to_texels: cell.cell_map[0],
                aa_scale: cell.cell_map[3],
                arm_points: self.marker_arm_points,
                padding: 0.0,
            };
        }
        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
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
            // Bracket all lattice preparation, starting before the first
            // optional shadow/ink/glow pass and ending after optional bloom.
            // The final egui composite belongs to paint's host pass.
            // Skipped while a readback is still in flight, so the query set is
            // never overwritten mid-cycle.
            let timing =
                self.drives_timer() && resources.timer.as_ref().is_some_and(GpuTimer::arming);
            if timing {
                resources.timer.as_ref().expect("armed timer").opening(egui_encoder);
            }

            // The shadow atlas, ahead of the scene pass that samples it: every
            // caster's own ink into its own cell, then the blur over the cells.
            // Present exactly while something casts this frame
            // (`PaneTargets::shadow`), so a frame with the Shadow shut pays
            // none of it and its draws multiply by 1.
            //
            // THREE draws into one cleared target, in the order the buffers sit
            // in rather than the picture's. Their cells are disjoint, so this
            // order decides nothing.
            let atlas = offscreen.shadow.as_ref().filter(|_| pane.box_count > 0);
            if let Some(atlas) = atlas {
                let mut pass = atlas.ink_pass(egui_encoder);
                if let Some(glyphs) =
                    pane.glyph_bind_group.as_ref().filter(|_| pane.glyph_count > 0)
                {
                    pass.set_pipeline(&resources.glyph_distance_pad_pipeline);
                    pass.set_bind_group(0, glyphs, &[]);
                    pass.set_vertex_buffer(0, pane.box_buffer.slice(..));
                    pass.draw(0..4, 0..pane.box_count);
                }
                if pane.instance_count > 0 {
                    pass.set_pipeline(&resources.node_cell_pipeline);
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                    pass.set_vertex_buffer(1, pane.node_cell_buffer.slice(..));
                    pass.draw(0..4, 0..pane.instance_count);
                }
                // ONE cross for the whole field, at the home sheet's size and
                // at level 1: every marker reads this same cell and spends its
                // own opacity where it reads it.
                let marker_has_cells =
                    packed.boxes.first().is_some_and(|b| b.cell[2] > 0.0 && b.cell[3] > 0.0);
                if pane.plus_count > 0 && self.marker_arm_points > 0.0 && marker_has_cells {
                    pass.set_pipeline(&resources.plus_cell_pipeline);
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    pass.draw(0..4, 0..1);
                }
                if let Some(glyphs) =
                    pane.glyph_bind_group.as_ref().filter(|_| pane.glyph_count > 0)
                {
                    pass.set_bind_group(0, glyphs, &[]);
                    pass.set_vertex_buffer(0, pane.glyph_buffer.slice(..));
                    pass.set_vertex_buffer(1, pane.cell_buffer.slice(..));
                    // Every name in a frame is one group, so the fill is one
                    // pipeline for the whole run — where a NODE's fill branches
                    // per box, its own cell draw serving both kinds.
                    pass.set_pipeline(if self.shadow.lattice_text.kernel.is_distance() {
                        &resources.glyph_distance_cell_pipeline
                    } else {
                        &resources.glyph_coverage_cell_pipeline
                    });
                    pass.draw(0..4, 0..pane.glyph_count);
                }
                drop(pass);
                // The same answer the atlas was allocated on, so the chain and
                // the plane it ping-pongs through can never disagree.
                if blurs {
                    let cells = &resources.shadow_cell_pipelines;
                    atlas.blur(
                        egui_encoder,
                        (&cells.blur_x, &cells.blur_y),
                        &pane.box_buffer,
                        pane.box_count,
                    );
                }
            }

            // The node glow, into a target of its own and BEFORE the scene
            // pass, which composites it at its bottom and samples it per node.
            //
            // One draw over the whole instance buffer, every sheet at once:
            // the blend the pass writes is commutative and never subtracts, so
            // there is nothing for a per-sheet walk to decide —
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

                // Cleared to transparent, which is the screen blend's own
                // identity: a screen over nothing is the source.
                let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("lattice_glow_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &glow.view,
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
                // Every node at once, in whatever order the instance buffer
                // holds them. The blend is commutative and never subtracts, so
                // this target is one field of light with no depth in it at all
                // — which is what makes it safe to lay under every sheet as a
                // single layer.
                if pane.instance_count > 0 {
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    pass.set_bind_group(1, &glow.strip.blurred_bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                    pass.set_pipeline(&resources.glow_pipeline);
                    pass.draw(0..4, 0..pane.instance_count);
                }
            }

            let scene = &resources.scenes[usize::from(offscreen.bloom.is_some())];
            let attachment = |view| {
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
            let attachments = [
                attachment(&offscreen.color_view),
                offscreen.bloom.as_ref().and_then(|b| attachment(&b.nodes_view)),
            ];
            let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lattice_scene_pass"),
                color_attachments: &attachments[..if offscreen.bloom.is_some() { 2 } else { 1 }],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // The light, FIRST — under every node, marker and label in the
            // pass, which is what makes a node a lamp rather than a hole and
            // what puts the light under every SHADOW: each item multiplies
            // whatever is already in the frame under it by its own blurred ink
            // (`node_paint`, `plus_paint`, text.wgsl's `fs_shadow_box`), and
            // the light is in the frame first.
            //
            // With bloom on it writes both attachments, so the bloom's
            // bright pass reads the light exactly as it reads the nodes: it is
            // light the nodes emit, and it blooms with the rest of them.
            if let Some(glow) = offscreen.glow.as_ref() {
                pass.set_pipeline(&scene.glow_over);
                pass.set_bind_group(0, &glow.bind_group, &[]);
                pass.draw(0..4, 0..1);
            }

            // That same target at group 1 of every node and marker draw, for
            // the wash to read back. The dummy where the light does not exist
            // at all, which is the Reach bar at 0 — a transparent read is the
            // plain ground, so nothing branches (see `glow_dummy_bind_group`).
            let light =
                offscreen.glow.as_ref().map_or(&resources.glow_dummy_bind_group, |g| &g.bind_group);
            // The finished atlas at group 2 of every node and marker draw, for
            // each to read its own cell. The 1x1 stand-in where this frame
            // packed none: every box is then zeros, and a caster with no cell
            // multiplies by exactly 1 with nothing sampled (`shadow_through`).
            let cells = atlas.map_or(&resources.shadow_dummy_bind_group, |a| a.read());

            // The order, as `from_scene` laid it down (see [`Draw`]). One walk
            // forward, back to front, with nothing here deciding what goes
            // where — a name is covered by exactly what covers its node, and a
            // cross covers the nodes behind it and no others, because that is
            // where each of them was emitted.
            for draw in &pane.draws {
                match *draw {
                    Draw::Nodes(a, b) => {
                        pass.set_pipeline(&scene.nodes);
                        pass.set_bind_group(0, &pane.bind_group, &[]);
                        pass.set_bind_group(1, light, &[]);
                        pass.set_bind_group(2, cells, &[]);
                        pass.set_bind_group(3, &pane.caster_bind_group, &[]);
                        pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                        // One box per node instance: what this draw takes off
                        // a box is the caster's INDEX, which is the only row of
                        // it the scene pass reads.
                        pass.set_vertex_buffer(1, pane.node_cell_buffer.slice(..));
                        pass.draw(0..4, a..b);
                    }
                    Draw::Pluses(a, b) => {
                        pass.set_pipeline(&scene.pluses);
                        pass.set_bind_group(0, &pane.bind_group, &[]);
                        pass.set_bind_group(1, light, &[]);
                        pass.set_bind_group(2, cells, &[]);
                        pass.set_bind_group(3, &pane.caster_bind_group, &[]);
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
                        // One instance at the caster's own index, which is how
                        // the draw finds its shadow: `vs_shadow_box` reads the
                        // quad and the level out of the array at group 3 and
                        // binds no vertex buffer at all.
                        if let Some(atlas) = atlas.filter(|_| (l as usize) < pane.caster_count) {
                            pass.set_bind_group(2, atlas.read(), &[]);
                            pass.set_bind_group(3, &pane.caster_bind_group, &[]);
                            pass.set_pipeline(&scene.shadow_box);
                            pass.draw(0..4, l..l + 1);
                        }
                        pass.set_vertex_buffer(0, pane.glyph_buffer.slice(..));
                        pass.set_pipeline(&scene.glyph_fill);
                        pass.draw(0..4, a..b);
                    }
                }
            }
            drop(pass);

            if let Some(bloom) = &offscreen.bloom {
                bloom.chain.run(egui_encoder, Self::bloom_pipelines(resources), "lattice");
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

mod gpu_harness;
#[cfg(test)]
mod lattice_tests;
