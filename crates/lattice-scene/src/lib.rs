//! The scene layer: turns core state (note tracker + tuning) into a
//! render-friendly description, once per frame. The renderer consumes a
//! [`Scene`] and knows nothing about MIDI; the core knows nothing about
//! cameras or colors. Animation/envelope *policy* lives here.

pub mod skin;

use glam::{Mat4, Vec2, Vec3, Vec4};
use lattice_core::{coords, ChannelRole, LatticePos, NoteTracker, Tuning};

/// Axis mapping, matching v1's orientation: major thirds run horizontally
/// (x), fifths vertically (y), and harmonic sevenths in depth (z).
fn lattice_to_world(pos: LatticePos, spacing: f32) -> Vec3 {
    Vec3::new(
        pos.fives as f32 * spacing,
        pos.threes as f32 * spacing,
        pos.sevens as f32 * spacing,
    )
}

/// The OUTER layer: how a node shows which octaves its pitch class is
/// sounding in. Every style draws its glyphs inside the radial band
/// between the view's `outer_inner` and `outer_outer` radii — the band is
/// the glyph set's radial footprint, so switching styles keeps the octave
/// display the same size. The fragment shader draws the glyphs from the
/// per-node octave bitmask.
///
/// Fully independent of the CORE layer ([`CoreStyle`]): the glyphs draw
/// the same whatever the core does. The [`ViewConfig::outer_backdrop`]
/// toggle — its own outer-layer setting, not the core's business — adds a
/// cohesion device so a single sounding octave still reads as one whole
/// note (Slices ghosts its silent slots in the note color, completing the
/// circle; Dots strings its glyphs on a faint hoop; a Rings glyph is a
/// closed circle already, so it is unaffected).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OuterStyle {
    /// No octave indication.
    Off,
    /// Dots filling the band (orbit at its center, radius half its
    /// width); angle tracks absolute pitch (middle C straight up, 45deg
    /// clockwise per octave, pitch class within the octave included). All
    /// other styles keep this angle convention and change only the glyph
    /// shape. The aliases absorb the removed Petals and Flares styles, and
    /// the merged Bumps style (rim-seated blobs, too close to Dots to keep
    /// as its own row), so old persisted view blobs keep loading.
    #[default]
    #[serde(alias = "Petals", alias = "Flares", alias = "Bumps")]
    Dots,
    /// Annular pizza-slice sectors spanning the band, with
    /// constant-thickness gaps between neighbors.
    Slices,
    /// One concentric ring per octave, lowest innermost, spread across
    /// the band.
    Rings,
}

/// How the core orb is painted while notes sound (inert without an orb —
/// [`CoreStyle::Glow`] or [`CoreStyle::None`]). All styles share the same
/// instance data
/// (activation + per-note phase); the fragment/vertex shader switches
/// on a uniform. Kept as switchable candidates for live comparison — idle
/// nodes look identical in every style.
///
/// The aliases on Steady absorb node styles that used to exist (Breathe,
/// Sparks, and the Wire/Corona/Plasma/Aurora/Marble/Lava/Filament/Stripes/
/// Rings/Tiles set trimmed later) so persisted view blobs that still name
/// them keep loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum NodeStyle {
    /// The original look: steady disc + glow.
    #[default]
    #[serde(
        alias = "Breathe",
        alias = "Sparks",
        alias = "Wire",
        alias = "Corona",
        alias = "Plasma",
        alias = "Aurora",
        alias = "Marble",
        alias = "Lava",
        alias = "Filament",
        alias = "Stripes",
        alias = "Rings",
        alias = "Tiles"
    )]
    Steady,
    /// Gas ball: octave colors sheared into rotating spiral streaks, like
    /// stirred paint.
    Vortex,
    /// Pattern: soft checkerboard on the globe graticule.
    Checker,
    /// Pattern: two-armed spiral of color waves hugging the sphere.
    Spiral,
    /// Pattern: beach-ball sectors around a tilted pole, slowly turning.
    Pinwheel,
}

impl NodeStyle {
    /// Index used by the shader (uniform `misc.w`). Indices are preserved
    /// from the original 15-style set so each kept style's shader branch in
    /// lattice.wgsl stays byte-for-byte unchanged; the gaps are the removed
    /// styles.
    pub fn shader_index(self) -> u32 {
        match self {
            NodeStyle::Steady => 0,
            NodeStyle::Vortex => 3,
            NodeStyle::Pinwheel => 11,
            NodeStyle::Spiral => 12,
            NodeStyle::Checker => 13,
        }
    }

    /// The field family — everything except Steady: styles whose active
    /// discs paint the swirled octave-color field (noise-driven gas or
    /// deterministic patterns). These animate on global time with a stable
    /// per-node seed (see [`derive_scene`]), so note events never restart
    /// the pattern. Mirrors `is_field_style` in lattice.wgsl; keep in sync.
    pub fn is_field_style(self) -> bool {
        !matches!(self, NodeStyle::Steady)
    }
}

impl OuterStyle {
    /// Index used by the shader (uniform `misc.z`). Indices are preserved
    /// from the original set so the kept glyphs' shader branches stay
    /// unchanged; 2/3 were the removed Petals/Flares, 4 the merged Bumps.
    pub fn shader_index(self) -> u32 {
        match self {
            OuterStyle::Off => 0,
            OuterStyle::Dots => 1,
            OuterStyle::Slices => 5,
            OuterStyle::Rings => 6,
        }
    }
}

/// Legacy load-only core mode, from before the core became a plain radius
/// (`core_radius`, with 0 = off) plus a `core_solidity` slider. Persisted
/// blobs still carry a `core_style` token; [`ViewConfig::migrate_legacy`]
/// folds each value into today's radius/solidity and then this is ignored
/// (the field is `skip_serializing`). Kept as a distinct type only so those
/// tokens keep deserializing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CoreStyle {
    /// The pre-radius-off "nothing" mode (serialized `"Empty"`). Folds to
    /// `core_radius = 0` (off). The bare `"None"` token is the older
    /// glow-only mode instead, aliased onto [`Glow`](CoreStyle::Glow).
    #[serde(rename = "Empty")]
    None,
    /// The core is present with radius + solidity already in their own
    /// fields; nothing to fold. The default (and what recent blobs wrote).
    #[default]
    On,
    /// The pre-solidity solid orb. Folds to solidity 1.
    Orb,
    /// The pre-solidity glow-only mode (also the bare `"None"` token
    /// pre-split blobs wrote). Folds to solidity 0.
    #[serde(alias = "None")]
    Glow,
}

/// The idle-node marker: a minimal grey mark shown at each home-sheet node
/// at all times, independent of the active appearance and of whether a note
/// is playing. Sized by [`ViewConfig::idle_radius`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum IdleMarker {
    /// Nothing.
    None,
    /// A filled grey dot.
    Dot,
    /// A thin grey outline circle (the classic placeholder look).
    #[default]
    Circle,
}

impl IdleMarker {
    /// Index the shader reads (uniform `misc4.w`): 0 none, 1 dot, 2 circle.
    pub fn shader_index(self) -> u32 {
        match self {
            IdleMarker::None => 0,
            IdleMarker::Dot => 1,
            IdleMarker::Circle => 2,
        }
    }
}

/// The short-lived NodeBody experiment's variants (one working-tree
/// build, 2026-07-18, octave-only note bodies): parsed load-only via
/// `ViewConfig::node_body` and folded into the core/outer split by
/// [`ViewConfig::migrate_legacy`], so blobs saved by that build keep
/// loading instead of dropping the whole persist.
/// (Not an Option: the legacy blobs wrote the variant bare, which RON
/// would refuse to parse into an Option's `Some`; the `Disc` default
/// doubles as the "nothing to migrate" state.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum LegacyNodeBody {
    #[default]
    Disc,
    #[serde(alias = "Pie")]
    Slices,
    Rings,
    Beads,
}

/// Purely-visual settings (not host-automatable parameters). The UI layer
/// persists these separately from plugin parameters.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ViewConfig {
    /// World-space distance between adjacent nodes.
    pub spacing: f32,
    /// Extent of the displayed lattice along each axis (± steps around the
    /// center).
    pub extent_threes: i32,
    pub extent_fives: i32,
    pub extent_sevens: i32,
    /// Center of the displayed window, in lattice steps from C (v1's Grid
    /// X/Y/Z). The center node renders at the world origin, so panning the
    /// window doesn't walk the content away from the camera.
    #[serde(default)]
    pub center_threes: i32,
    #[serde(default)]
    pub center_fives: i32,
    #[serde(default)]
    pub center_sevens: i32,
    /// Outer octave layer style. The alias keeps pre-rename blobs (field
    /// `octave_style`) loading; the default covers even older blobs.
    #[serde(default, alias = "octave_style")]
    pub outer_style: OuterStyle,
    /// Draw note-name labels on hovered and sounding nodes.
    /// serde(default) keeps older persisted blobs loadable.
    #[serde(default = "default_true")]
    pub show_labels: bool,
    /// Under each note-name label, also show the node's pitch class in
    /// cents. Only meaningful while `show_labels` is on.
    #[serde(default = "default_true")]
    pub show_cents: bool,
    /// How held notes are rendered (see NodeStyle).
    #[serde(default)]
    pub node_style: NodeStyle,
    /// Legacy load-only core mode (see [`CoreStyle`]); folded into
    /// `core_radius`/`core_solidity` by [`migrate_legacy`](Self::migrate_legacy)
    /// and never written back.
    #[serde(default, skip_serializing)]
    pub core_style: CoreStyle,
    /// The core's solidity, 0..1: a soft glow at 0, morphing continuously
    /// to the classic solid orb at 1 (the disc fades in over its glow
    /// skirt and its edge crisps). Inert while the core is off.
    #[serde(default = "default_core_solidity")]
    pub core_solidity: f32,
    /// The core's radius, in quad UV units (0 = node center, 1 = quad
    /// edge; the classic disc edge sits at 0.46). Sizes the disc and its
    /// glow together. **A radius of 0 turns the core off** — the outer
    /// octave glyphs then carry the note alone.
    #[serde(default = "default_core_radius")]
    pub core_radius: f32,
    /// The outer octave layer's radial band (same UV units): every outer
    /// style fits its glyphs' radial footprint to this. The aliases keep
    /// the previous build's slice_inner/slice_outer blobs loading;
    /// derive_scene keeps outer ahead of inner, so any dragged
    /// combination still renders a visible band.
    #[serde(default = "default_outer_inner", alias = "slice_inner")]
    pub outer_inner: f32,
    #[serde(default = "default_outer_outer", alias = "slice_outer")]
    pub outer_outer: f32,
    /// Outer-layer cohesion device (independent of the core): draw the
    /// silent octaves faintly behind the sounding glyphs so a lone octave
    /// still reads as a whole note — Slices ghosts its empty slots in the
    /// note color to complete the annulus, Dots strings its glyphs on a
    /// faint hoop. No effect on Rings (already closed circles) or Off. Was
    /// implicitly tied to "core off", then its own on/off toggle.
    ///
    /// Now an opacity, 0..1: 0 is off (exactly the old `false`) and 1 is
    /// the old `true`'s strength, with everything between available so the
    /// backdrop can sit as far under the sounding glyphs as you like. It
    /// scales the shader's built-in hoop/ghost levels rather than
    /// replacing them, so 1 reproduces the previous look byte for byte.
    /// Serialized under a new key, leaving the old one to the load-only
    /// [`legacy_outer_backdrop`](Self::legacy_outer_backdrop) bool.
    #[serde(default, rename = "outer_backdrop_alpha")]
    pub outer_backdrop: f32,
    /// Load-only shim: blobs from before the backdrop became an opacity
    /// stored it as a bool under `outer_backdrop`. Folded into
    /// `outer_backdrop` by [`migrate_legacy`](Self::migrate_legacy) and
    /// never written back. Without this the bool would fail to
    /// deserialize into the f32 above and take the WHOLE persist with it
    /// (the loader drops the blob on any parse error), losing the user's
    /// layout and camera too.
    #[serde(
        default,
        skip_serializing,
        rename = "outer_backdrop",
        deserialize_with = "bare_bool_as_some"
    )]
    pub legacy_outer_backdrop: Option<bool>,
    /// The outer glyphs' solidity, 0..1 (mirrors [`core_solidity`] for the
    /// octave layer): 1 draws crisp shapes (the classic look), and toward 0
    /// their soft edges spread until they melt into soft glowy marks. Only
    /// widens the glyph edges — shapes and angles are unchanged.
    #[serde(default = "default_outer_solidity")]
    pub outer_solidity: f32,
    // ---- Idle (unlit) node marker ----------------------------------------
    // A minimal grey marker at each home-sheet node, drawn ALWAYS —
    // independent of both the active appearance and whether a note is
    // playing there (an active note simply composites over it). Off-sheet
    // nodes draw nothing (the grid marks them).
    /// What the idle marker is (see [`IdleMarker`]): nothing, a filled dot,
    /// or an outline circle.
    #[serde(default)]
    pub idle_marker: IdleMarker,
    /// The idle marker's radius (dot fill / circle) in quad UV units.
    /// Independent of the active `core_radius`.
    #[serde(default = "default_idle_radius")]
    pub idle_radius: f32,
    /// Load-only shim: the short-lived NodeBody build's blobs set this,
    /// and [`ViewConfig::migrate_legacy`] folds it into core/outer. Never
    /// saved.
    #[serde(default, skip_serializing)]
    pub node_body: LegacyNodeBody,
    // ---- Home grid -------------------------------------------------------
    // The faint structural grid between node positions (see `derive_grid`).
    // Its look used to be fixed: the skin's color, a hardcoded inset, a
    // hardcoded thickness, dashes reserved for the sevens links.
    /// Grid line color, linear RGBA. Alpha is the idle line opacity — the
    /// strength an unlit home-sheet segment draws at — so this one picker
    /// covers both "what color" and "how faint". Defaults to the skin's
    /// `grid_line`, which is where this always came from; the skin has no
    /// runtime setter, so a user-chosen grid color has to live here.
    /// Lit segments still take their sounding notes' color.
    #[serde(default = "default_grid_color")]
    pub grid_color: [f32; 4],
    /// Grid line thickness as a multiple of the built-in width. 1 is the
    /// classic hairline; the shader scales its grid half-width by this.
    #[serde(default = "default_grid_thickness")]
    pub grid_thickness: f32,
    /// How far a grid segment stops short of each node center, as a factor
    /// of the node radius — the padding between a line end and the
    /// dot/circle drawn there. The old fixed 1.05 is the default: slightly
    /// wider than the disc's visual radius, so the gap fully contains a
    /// sounding note's circle. 0 runs the lines right into the centers.
    ///
    /// Under `grid_half_offset` the lines no longer reach any node, so
    /// this becomes the gap at the cell corners instead: 0 closes the
    /// cells into a continuous mesh, larger values open them into ticks.
    #[serde(default = "default_grid_inset")]
    pub grid_inset: f32,
    /// Draw the in-plane grid lines dashed. The sevens-axis links are
    /// always dashed regardless — that dash is what distinguishes a depth
    /// link from an in-sheet line, and isn't a style choice.
    #[serde(default)]
    pub grid_dashed: bool,
    /// Offset the in-plane grid by half a cell, so nodes sit in the middle
    /// of the grid squares instead of at their intersections. The segments
    /// (and so their lighting) are unchanged — only where they're drawn
    /// moves — which keeps the boundary exactly as complete as it is
    /// without the offset. The sevens links don't move: they connect
    /// actual notes across sheets, and shifting them off their nodes would
    /// break what they mean.
    #[serde(default)]
    pub grid_half_offset: bool,
    /// Light up lattice edges between simultaneously sounding adjacent
    /// nodes, so a chord's interval structure renders as geometry.
    #[serde(default)]
    pub show_chord_edges: bool,
    /// Meantone mode: lock the major-third tuning to four perfect fifths
    /// (temper out the syntonic comma). While on, the third-tuning value is
    /// derived from the fifth (in `root_ui`) and note-name labels drop
    /// their comma marks; Learn mode toggles this from the held chord.
    #[serde(default)]
    pub meantone: bool,
    /// Hide all dock chrome (tab bars; separators thin to a hairline) so
    /// adjacent panes — lattice above spectrum, in the default layout —
    /// record as one seamless surface. Esc restores.
    #[serde(default)]
    pub frameless: bool,
    /// Offscreen render resolution as a multiple of the pane's native pixel
    /// size: >1 supersamples (crisper glyph edges), <1 renders coarse and
    /// upscales. 1.0 reproduces the pre-offscreen-pass output exactly.
    #[serde(default = "default_render_scale")]
    pub render_scale: f32,
    /// Bloom post-process: how much blurred brightness gets added back
    /// as a halo around bright notes. 0 (the default) disables the chain
    /// entirely — the composite is then exactly the plain scene, so there
    /// is deliberately no separate on/off toggle.
    #[serde(default)]
    pub bloom_strength: f32,
}

fn default_true() -> bool {
    true
}

/// Read a legacy bare `true`/`false` into an `Option<bool>` that means
/// "the key was there". A plain `Option<bool>` field can't do this: RON
/// writes options as `Some(true)`/`None`, so it rejects the bare bool the
/// old blobs actually contain. `serde(default)` still supplies `None` when
/// the key is absent — this only runs when it is present.
fn bare_bool_as_some<'de, D>(d: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(d).map(Some)
}

/// The classic disc edge radius, from before the core was sizable.
fn default_core_radius() -> f32 {
    0.46
}

/// Solid orb by default — the classic look, and the identity end of the
/// solidity axis.
fn default_core_solidity() -> f32 {
    1.0
}

/// Crisp octave glyphs by default (the classic look, identity end of the
/// outer solidity axis).
fn default_outer_solidity() -> f32 {
    1.0
}

/// Idle marker at the classic disc radius, so the default (a Circle)
/// reproduces the old placeholder ring — now independent of the core and
/// of the playing state.
fn default_idle_radius() -> f32 {
    0.46
}

/// Outer band defaults: the classic Slices annulus (SLICE_IN/OUT before
/// the band was parameterized), matching the default outer style. Dots at
/// this band renders larger than v1's (which occupied 0.545..0.795 —
/// drag the bars there to recover that look exactly).
fn default_outer_inner() -> f32 {
    0.56
}

fn default_outer_outer() -> f32 {
    0.93
}

fn default_render_scale() -> f32 {
    1.0
}

/// The grid's color comes from the skin by default, which is the only
/// place it came from before the grid became customizable.
fn default_grid_color() -> [f32; 4] {
    skin::active_skin().grid_line.to_array()
}

fn default_grid_thickness() -> f32 {
    1.0
}

/// The fixed inset from before the grid became customizable: a factor of
/// the node radius, slightly larger than the disc's visual radius (~0.83 ×
/// radius, see the quad math in lattice.wgsl) so the gap fully contains
/// the circle a sounding note draws there, with a slim margin.
fn default_grid_inset() -> f32 {
    1.05
}

impl ViewConfig {
    /// Every lattice position the view currently displays. ALL consumers
    /// (scene derivation, spectral ticks, notes-pane mapping) must iterate
    /// this same set so "on the lattice" means one thing.
    pub fn visible_positions(&self) -> impl Iterator<Item = LatticePos> {
        coords::positions_within(
            self.center_threes - self.extent_threes..=self.center_threes + self.extent_threes,
            self.center_fives - self.extent_fives..=self.center_fives + self.extent_fives,
            self.center_sevens - self.extent_sevens..=self.center_sevens + self.extent_sevens,
        )
    }

    /// How many positions [`visible_positions`](Self::visible_positions)
    /// yields — the product of each axis's inclusive span — so per-frame
    /// buffers can preallocate instead of growing through reallocations.
    pub fn visible_count(&self) -> usize {
        ((2 * self.extent_threes + 1).max(0) as usize)
            * ((2 * self.extent_fives + 1).max(0) as usize)
            * ((2 * self.extent_sevens + 1).max(0) as usize)
    }

    /// The displayed window's center as a lattice position.
    pub fn center(&self) -> LatticePos {
        LatticePos::new(self.center_threes, self.center_fives, self.center_sevens)
    }

    /// Fold fields from older blob layouts into the current ones; call
    /// after deserializing a persisted view.
    ///
    /// The pre-radius-off core modes collapse onto today's `core_radius`
    /// (0 = off) plus `core_solidity`: the old `None` becomes radius 0
    /// (off), the old solid `Orb` becomes solidity 1, and the old glow-only
    /// mode (`Glow`, also the bare `"None"` token) becomes solidity 0. The
    /// one-build NodeBody experiment's octave-only bodies map onto that glow
    /// (solidity 0) plus the matching outer style with the backdrop on
    /// (Beads was dots-on-a-hoop, which is exactly what a Dots outer with
    /// the backdrop draws; its band radii rode the slice_inner/slice_outer
    /// fields, absorbed by the outer_inner/outer_outer aliases).
    pub fn migrate_legacy(&mut self) {
        // The backdrop's pre-opacity bool: on means full strength, which
        // is what that build drew.
        if let Some(on) = self.legacy_outer_backdrop.take() {
            self.outer_backdrop = if on { 1.0 } else { 0.0 };
        }

        match std::mem::replace(&mut self.core_style, CoreStyle::On) {
            CoreStyle::None => self.core_radius = 0.0,
            CoreStyle::Orb => self.core_solidity = 1.0,
            CoreStyle::Glow => self.core_solidity = 0.0,
            CoreStyle::On => {}
        }

        let outer = match std::mem::take(&mut self.node_body) {
            LegacyNodeBody::Disc => return,
            LegacyNodeBody::Slices => OuterStyle::Slices,
            LegacyNodeBody::Rings => OuterStyle::Rings,
            LegacyNodeBody::Beads => OuterStyle::Dots,
        };
        self.core_solidity = 0.0;
        self.outer_style = outer;
        self.outer_backdrop = 1.0;
    }
}

impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig {
            spacing: 1.0,
            // A tall window of fifths, a wide band of thirds, and one step
            // of sevenths out of the box.
            extent_threes: 10,
            extent_fives: 6,
            extent_sevens: 1,
            center_threes: 0,
            center_fives: 0,
            center_sevens: 0,
            outer_style: OuterStyle::Slices,
            show_labels: true,
            show_cents: true,
            node_style: NodeStyle::Checker,
            core_style: CoreStyle::default(),
            core_solidity: default_core_solidity(),
            core_radius: default_core_radius(),
            outer_inner: default_outer_inner(),
            outer_outer: default_outer_outer(),
            outer_backdrop: 0.0,
            legacy_outer_backdrop: None,
            outer_solidity: default_outer_solidity(),
            idle_marker: IdleMarker::default(),
            idle_radius: default_idle_radius(),
            node_body: LegacyNodeBody::Disc,
            grid_color: default_grid_color(),
            grid_thickness: default_grid_thickness(),
            grid_inset: default_grid_inset(),
            grid_dashed: false,
            grid_half_offset: false,
            show_chord_edges: false,
            meantone: false,
            frameless: false,
            render_scale: default_render_scale(),
            bloom_strength: 0.0,
        }
    }
}

/// Per-frame mirrors of the host-automatable appearance parameters. The
/// shell copies these from its param backend every frame (see root_ui).
/// Deliberately NOT part of [`ViewConfig`] or the persist blob: the param
/// system owns these values, and persisting a copy would create a second
/// source of truth that's dead on arrival at load time.
#[derive(Clone, Copy, Debug)]
pub struct FrameParams {
    /// Seconds a released note's pitch class keeps fading.
    pub pitch_class_fade_time: f32,
    /// Seconds an octave indicator keeps fading after release; independent
    /// of the note highlight.
    pub octave_fade_time: f32,
    /// Pitch (MIDI note) mapped to the darkest gradient color on
    /// pitch-gradient channels.
    pub darkest_pitch: f32,
    /// Pitch mapped to the brightest gradient color.
    pub brightest_pitch: f32,
}

impl Default for FrameParams {
    fn default() -> Self {
        FrameParams {
            pitch_class_fade_time: 1.0,
            octave_fade_time: 1.0,
            darkest_pitch: 24.0,
            brightest_pitch: 108.0,
        }
    }
}

/// How the camera maps depth to the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Projection {
    /// Classic perspective: farther content converges and shrinks.
    #[default]
    Perspective,
    /// Orthographic ("isometric-style"): uniform scale at every depth, so
    /// equal intervals render at equal screen offsets everywhere and
    /// parallel lattice lines stay parallel. Depth then reads only
    /// through the deliberate cues (node depth-scale, occlusion).
    Orthographic,
    /// Cabinet (oblique): the camera faces the fifths/thirds sheet
    /// straight on (orbit is ignored; the UI turns plain drags into
    /// pans), which renders that primary plane with zero distortion, and
    /// the sevens axis shears to a uniform screen offset — every
    /// seventh-step is the same arrow anywhere on screen. Direction and
    /// length of that arrow are [`Camera::cabinet_angle`] and
    /// [`Camera::cabinet_scale`].
    Cabinet,
}

fn default_cabinet_angle() -> f32 {
    45f32.to_radians()
}

fn default_cabinet_scale() -> f32 {
    0.6
}

/// Simple orbit camera. Angles in radians.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
    /// serde(default) keeps pre-projection persisted blobs loadable.
    #[serde(default)]
    pub projection: Projection,
    /// Cabinet only: on-screen direction of the sevens axis, radians
    /// counterclockwise from screen-right (drafting convention picks 30°,
    /// 45°, or 60°; default 45°).
    #[serde(default = "default_cabinet_angle")]
    pub cabinet_angle: f32,
    /// Cabinet only: screen length of one seventh-step as a fraction of a
    /// front-plane step (0.5 = classic cabinet, 1.0 = cavalier).
    #[serde(default = "default_cabinet_scale")]
    pub cabinet_scale: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: Vec3::ZERO,
            yaw: 0.4,
            pitch: 0.3,
            distance: 12.0,
            fov_y: 45f32.to_radians(),
            projection: Projection::Cabinet,
            cabinet_angle: default_cabinet_angle(),
            cabinet_scale: default_cabinet_scale(),
        }
    }
}

impl Camera {
    /// Yaw/pitch radians per dragged pixel.
    const ORBIT_SPEED: f32 = 0.01;
    /// Pan speed per pixel, scaled by distance so a pan feels the same at
    /// any zoom.
    const PAN_SPEED: f32 = 0.0016;
    /// Multiplicative zoom rate per scroll unit.
    const ZOOM_RATE: f32 = 0.002;
    /// Keep pitch shy of the poles so look_at's up vector stays valid.
    pub const PITCH_LIMIT: f32 = 1.5;
    /// Zoom clamp; CLIP_NEAR/CLIP_FAR must bracket it with margin.
    pub const MIN_DISTANCE: f32 = 2.0;
    pub const MAX_DISTANCE: f32 = 80.0;

    /// Orbit around the target by a drag delta in pixels.
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * Self::ORBIT_SPEED;
        self.pitch = (self.pitch + delta.y * Self::ORBIT_SPEED)
            .clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    /// Pan the target by a drag delta in pixels, grab-style: the content
    /// follows the pointer.
    pub fn pan(&mut self, delta: Vec2) {
        let (right, up) = self.right_up();
        let k = self.distance * Self::PAN_SPEED;
        self.target += up * (delta.y * k) - right * (delta.x * k);
    }

    /// Zoom by a scroll delta (positive = closer), clamped to the working
    /// range.
    pub fn zoom(&mut self, scroll: f32) {
        self.distance = (self.distance * (1.0 - scroll * Self::ZOOM_RATE))
            .clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    /// Zoom by a multiplicative factor (>1 = closer), clamped to the working
    /// range. This is the pinch/zoom-gesture counterpart to [`Self::zoom`]:
    /// egui reports trackpad pinches and modifier+scroll as a `zoom_delta`
    /// factor rather than a scroll delta, and the lattice honors both.
    pub fn zoom_by(&mut self, factor: f32) {
        if factor > 0.0 {
            self.distance =
                (self.distance / factor).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
        }
    }

    pub fn eye(&self) -> Vec3 {
        // Cabinet is a fixed-viewpoint projection: the eye always faces
        // the fifths/thirds sheet straight on, whatever yaw/pitch say (they
        // keep their values for when the user switches back).
        let dir = if self.projection == Projection::Cabinet {
            Vec3::Z
        } else {
            Vec3::new(
                self.pitch.cos() * self.yaw.sin(),
                self.pitch.sin(),
                self.pitch.cos() * self.yaw.cos(),
            )
        };
        self.target + dir * self.distance
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        // Both glam _rh constructors produce 0..1 clip-space depth, which
        // is what wgpu expects.
        let aspect = aspect.max(0.01);
        let proj = match self.projection {
            Projection::Perspective => {
                Mat4::perspective_rh(self.fov_y, aspect, CLIP_NEAR, CLIP_FAR)
            }
            // The ortho window is the perspective frustum's cross-section
            // at the target, so toggling projections keeps the framing at
            // the focus plane, and zoom (distance) keeps scaling the view.
            Projection::Orthographic => {
                let half_h = self.distance * (self.fov_y * 0.5).tan();
                let half_w = half_h * aspect;
                Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, CLIP_NEAR, CLIP_FAR)
            }
            // Orthographic window plus a shear: view-space depth relative
            // to the focus plane becomes a screen offset along
            // `cabinet_angle` scaled by `cabinet_scale`. The focus plane
            // itself is unmoved (the shear's translation term cancels
            // there), so framing still matches the other projections.
            Projection::Cabinet => {
                let half_h = self.distance * (self.fov_y * 0.5).tan();
                let half_w = half_h * aspect;
                let ortho =
                    Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, CLIP_NEAR, CLIP_FAR);
                let (sin, cos) = self.cabinet_angle.sin_cos();
                let (kx, ky) = (self.cabinet_scale * cos, self.cabinet_scale * sin);
                let mut shear = Mat4::IDENTITY;
                shear.z_axis.x = kx;
                shear.z_axis.y = ky;
                shear.w_axis.x = kx * self.distance;
                shear.w_axis.y = ky * self.distance;
                ortho * shear
            }
        };
        proj * self.view()
    }

    /// Camera-space right/up axes in world space, for billboarding.
    pub fn right_up(&self) -> (Vec3, Vec3) {
        let view = self.view();
        // Rows of the rotation part of the view matrix.
        let right = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);
        (right, up)
    }
}

/// Camera clip planes; must bracket Camera::MIN/MAX_DISTANCE with margin.
const CLIP_NEAR: f32 = 0.1;
const CLIP_FAR: f32 = 200.0;

/// Node radius as a fraction of the lattice spacing.
const NODE_RADIUS_FACTOR: f32 = 0.25;

/// Octave indicator slots (MIDI octaves 0..=9).
pub const OCTAVE_SLOTS: usize = 10;

/// Seconds an octave indicator eases in after note-on. Keeps a fresh
/// octave's color GROWING into the gas swirl instead of instantly
/// repainting its share of the disc (and softens dots-mode pop-in);
/// short enough to still feel immediate.
const OCTAVE_ATTACK_TIME: f64 = 0.15;

/// Samples in the pitch->color lookup the dots octave style uses to tint
/// each dot by its own octave's pitch. The shader mirrors this length.
pub const DOT_RAMP_N: usize = 16;

/// One lattice node, ready for instanced rendering.
#[derive(Clone, Copy, Debug)]
pub struct NodeInstance {
    pub lattice_pos: LatticePos,
    pub world_pos: Vec3,
    /// RGBA base color (alpha unused for now).
    pub color: Vec4,
    /// 0 = idle, 1 = fully lit. Held notes are 1; released notes decay.
    pub activation: f32,
    /// Per-octave activation (slot = MIDI octave 0..=9, clamped): each
    /// octave's indicator fades on its own voice's envelope, independent of
    /// the node's overall activation.
    pub octaves: [f32; OCTAVE_SLOTS],
    /// Seconds since the strongest voice lighting this node started; 0
    /// when idle. Computed in f64 and handed to the shader small: absolute
    /// f32 seconds lose millisecond precision within a day of DAW uptime,
    /// which would visibly quantize the animation.
    pub age: f32,
    /// Small constant seeding animation variety. NOT a timestamp — only
    /// ever used as a seed. Per-note (derived from the note-on time,
    /// wrapped) for age-driven styles; a stable per-node hash for field
    /// styles (see [`NodeStyle::is_field_style`]).
    pub seed: f32,
    /// Render as an outline instead of a filled disc (channel 14, v1's
    /// "channel 15" in MIDI convention).
    pub outlined: bool,
    pub hovered: bool,
    /// Depth-cue size multiplier (see [`depth_scale`]): nodes nearer the
    /// eye than the camera's focus distance grow, farther ones shrink,
    /// exaggerating the perspective so depth reads at a glance.
    pub scale: f32,
    /// On the home (center sevens) sheet. Home nodes keep a blank
    /// placeholder ring while idle; off-sheet nodes draw nothing.
    pub on_home: bool,
    /// The node's pitch class in cents under the current tuning, for the
    /// in-lattice cents readout.
    pub cents: f32,
}

impl NodeInstance {
    /// Whether this node puts anything on screen, and so can carry pitch
    /// info (hover label, tuning readout). Sounding nodes always draw;
    /// idle ones only on the home sheet, where they keep a placeholder
    /// marker. Off-sheet idle nodes draw literally nothing, so revealing
    /// their pitch on mouse-over would be information from nowhere.
    ///
    /// Deliberately ignores [`Scene::idle_marker`] being `None`: that
    /// setting hides the idle markers but shouldn't make the home sheet
    /// unhoverable, which would leave an empty lattice uninspectable.
    pub fn is_visible(&self) -> bool {
        self.activation > 0.0 || self.on_home
    }
}

/// A glowing beam between two simultaneously sounding, lattice-adjacent
/// nodes (one unit step along exactly one prime axis = one interval).
#[derive(Clone, Copy, Debug)]
pub struct EdgeInstance {
    pub a: Vec3,
    pub b: Vec3,
    pub color: Vec4,
    /// min of the two nodes' activations: the beam fades with whichever
    /// endpoint fades first. (Grid segments: line opacity.)
    pub strength: f32,
    /// Grid segments only: render as short dashes (the sevens-axis links
    /// between sheets). Never set on chord beams.
    pub dashed: bool,
}

/// Everything the renderer needs for one frame.
pub struct Scene {
    pub nodes: Vec<NodeInstance>,
    pub camera: Camera,
    /// Seconds for global shader animation, wrapped hourly so f32
    /// precision holds in long sessions. The field styles clock on this so
    /// their fields keep flowing across note events (at worst the pattern
    /// jumps once an hour at the wrap); age-driven styles use
    /// [`NodeInstance`]'s `age`/`seed` instead (unwrapped age math).
    pub time: f32,
    /// Base node radius in world units (scales with lattice spacing).
    pub node_radius: f32,
    pub outer_style: OuterStyle,
    pub node_style: NodeStyle,
    /// The core's radius in quad UV units; `0` turns the core off (nothing
    /// at all). Sizes both the disc and its glow; the shader reads solidity
    /// separately (`core_solidity`).
    pub core_radius: f32,
    /// The core's solidity 0..1 (see [`ViewConfig::core_solidity`]): 0 a
    /// soft glow, 1 the solid orb. Ignored when the core is off.
    pub core_solidity: f32,
    /// The outer octave layer's radial band (quad UV units), already
    /// sanitized: outer is always ahead of inner.
    pub outer_inner: f32,
    pub outer_outer: f32,
    /// Outer-layer cohesion device (see [`ViewConfig::outer_backdrop`]):
    /// ghost the silent octaves faintly to complete the ring. Independent
    /// of the core. Already clamped to 0..1; 0 draws no backdrop at all.
    pub outer_backdrop: f32,
    /// The outer glyphs' solidity 0..1 (see [`ViewConfig::outer_solidity`]):
    /// 1 crisp, toward 0 the glyph edges spread into soft glows.
    pub outer_solidity: f32,
    /// The idle (unlit home-sheet node) marker, independent of the active
    /// appearance and of the playing state; drawn in the idle grey and
    /// composited under any active note. See [`ViewConfig::idle_marker`].
    pub idle_marker: IdleMarker,
    pub idle_radius: f32,
    /// Chord edges (empty when the toggle is off).
    pub edges: Vec<EdgeInstance>,
    /// The faint background grid (see [`derive_grid`]): one segment per
    /// adjacent pair of visible positions, inset so every node position
    /// keeps a circular gap where its disc draws while sounding. Segments
    /// between two sounding notes light up with their blended color.
    /// Reuses [`EdgeInstance`]; `strength` carries the line opacity.
    pub grid: Vec<EdgeInstance>,
    /// Grid line thickness as a multiple of the shader's built-in grid
    /// width (see [`ViewConfig::grid_thickness`]), already clamped.
    pub grid_thickness: f32,
    /// Pitch->color lookup for the dots octave style, matching the disc
    /// gradient; the renderer hands it to the shader (see [`pitch_ramp_lut`]).
    pub dot_ramp: [Vec4; DOT_RAMP_N],
    /// Gradient endpoints (MIDI notes) the shader maps a dot's pitch through
    /// to index `dot_ramp`; mirror the disc coloring's `FrameParams`.
    pub darkest_pitch: f32,
    pub brightest_pitch: f32,
    /// Offscreen render resolution multiplier (see [`ViewConfig`]); the
    /// renderer sizes its offscreen color+depth target by this.
    pub render_scale: f32,
    /// Bloom intensity; 0 disables the whole post-process chain.
    pub bloom_strength: f32,
}

fn lch(l: f64, c: f64, h: f64) -> Vec4 {
    // The conversion is unclamped and out-of-gamut LCH inputs yield values
    // outside 0..255 (v1's graphics stack clamped downstream; we must do it
    // ourselves before handing colors to the shader).
    let rgb = color_space::Rgb::from(color_space::Lch::new(l, c, h));
    Vec4::new(
        (rgb.r.clamp(0.0, 255.0) / 255.0) as f32,
        (rgb.g.clamp(0.0, 255.0) / 255.0) as f32,
        (rgb.b.clamp(0.0, 255.0) / 255.0) as f32,
        1.0,
    )
}

/// Normalized pitch height in 0..1 across the gradient range: 0 at
/// `darkest_pitch`, 1 at `brightest_pitch` (both MIDI note numbers).
fn pitch_ramp_t(pitch: f32, darkest_pitch: f32, brightest_pitch: f32) -> f64 {
    f64::from(
        (pitch.clamp(darkest_pitch, brightest_pitch) - darkest_pitch)
            / (brightest_pitch - darkest_pitch).max(0.01),
    )
}

/// The pitch-gradient LCH ramp as a function of normalized height `t`
/// (0..1). Shared by the node disc color and the dots octave style's
/// per-dot tint, so a dot is the same color as the disc that pitch lights.
fn pitch_ramp_lch(t: f64) -> Vec4 {
    lch(t * 80.0, 85.0 - t * 60.0, (-100.0 + t * 190.0).rem_euclid(360.0))
}

/// The pitch ramp sampled into [`DOT_RAMP_N`] colors evenly spaced over the
/// full `t` range, for the shader's per-dot color lookup (the shader maps a
/// dot's pitch to a `t` and indexes this). Endpoints of the disc gradient
/// are applied shader-side, so this LUT itself is range-independent.
pub fn pitch_ramp_lut() -> [Vec4; DOT_RAMP_N] {
    // Constant (range-independent, per the doc above) but each entry costs
    // several transcendentals through the LCH->sRGB conversion, and it's read
    // once per animating frame (~66/s). Compute it once and copy out the
    // cached array — same value, none of the per-frame color math.
    static LUT: std::sync::OnceLock<[Vec4; DOT_RAMP_N]> = std::sync::OnceLock::new();
    *LUT.get_or_init(|| {
        std::array::from_fn(|k| pitch_ramp_lch(k as f64 / (DOT_RAMP_N - 1) as f64))
    })
}

/// Ported verbatim from v1 (`editor/color.rs`); the channel policy itself
/// lives in [`ChannelRole`]. Gradient channels are colored by pitch height
/// on an LCH ramp between `darkest_pitch` and `brightest_pitch` (MIDI note
/// numbers).
pub fn channel_color(channel: u8, pitch: f32, darkest_pitch: f32, brightest_pitch: f32) -> Vec4 {
    match ChannelRole::of(channel) {
        ChannelRole::FixedColor => match channel {
            0 => lch(48.0, 45.0, 32.0),  // red
            1 => lch(65.0, 60.0, 68.0),  // orange
            2 => lch(80.0, 42.0, 83.0),  // yellow
            3 => lch(65.0, 50.0, 120.0), // green
            4 => lch(60.0, 40.0, 280.0), // blue
            5 => lch(50.0, 55.0, 305.0), // purple
            6 => lch(70.0, 30.0, 340.0), // pink
            7 => lch(80.0, 0.0, 0.0),    // white
            _ => lch(0.0, 0.0, 0.0),     // 8: black
        },
        ChannelRole::PitchGradient => {
            pitch_ramp_lch(pitch_ramp_t(pitch, darkest_pitch, brightest_pitch))
        }
        // Outline voices get a bright neutral (the ring shape is the
        // signal). Ignored never reaches here — the tracker drops it.
        ChannelRole::Outline | ChannelRole::Ignored => Vec4::new(0.85, 0.85, 0.88, 1.0),
    }
}

/// Build the frame's scene. `hovered` comes from last frame's picking (the
/// usual immediate-mode one-frame latency, invisible in practice).
pub fn derive_scene(
    tracker: &NoteTracker,
    tuning: &Tuning,
    view: &ViewConfig,
    frame: &FrameParams,
    camera: Camera,
    hovered: Option<LatticePos>,
    now: f64,
) -> Scene {
    let mut nodes = Vec::with_capacity(view.visible_count());
    let center = view.center();
    let eye = camera.eye();

    for pos in view.visible_positions() {
        let node_pc = tuning.pitch_class(pos);

        let mut activation = 0.0f32;
        let mut octaves = [0f32; OCTAVE_SLOTS];
        let mut color = skin::active_skin().node_idle;
        let mut outlined = false;
        let mut age = 0.0f32;
        let mut seed = 0.0f32;

        // O(nodes × voices); fine at this scale. If extents grow large,
        // index voices by quantized pitch class instead.
        for voice in tracker.voices() {
            if tuning.matches(voice.pitch_class, node_pc) {
                let a = voice.activation(now, frame.pitch_class_fade_time);
                if a > activation {
                    activation = a;
                    color = channel_color(
                        voice.channel,
                        voice.pitch,
                        frame.darkest_pitch,
                        frame.brightest_pitch,
                    );
                    outlined = ChannelRole::of(voice.channel) == ChannelRole::Outline;
                    // Subtract in f64, then narrow: both endpoints are
                    // large after long sessions; only the difference is
                    // safe as f32.
                    age = (now - voice.on_time).max(0.0) as f32;
                    seed = (voice.on_time % 256.0) as f32;
                }
                let slot = voice.octave.clamp(0, OCTAVE_SLOTS as i8 - 1) as usize;
                // Smoothstep ease-in over the first OCTAVE_ATTACK_TIME;
                // release still fades on the octave envelope.
                let a = ((now - voice.on_time) / OCTAVE_ATTACK_TIME).clamp(0.0, 1.0) as f32;
                let attack = a * a * (3.0 - 2.0 * a);
                octaves[slot] = octaves[slot]
                    .max(voice.activation(now, frame.octave_fade_time) * attack);
            }
        }

        // Field styles animate one continuous field per node — global time
        // as the clock plus a stable per-node seed — so pressing,
        // retriggering, or stacking notes lights the flow up without ever
        // restarting or reshuffling it. Age-driven styles keep the
        // per-note seed (wire's tumble phases).
        let seed = if view.node_style.is_field_style() { node_seed(pos) } else { seed };

        // World positions are relative to the window center, keeping the
        // displayed region under the camera wherever the window pans.
        let centered = pos - center;
        let world_pos = lattice_to_world(centered, view.spacing);
        nodes.push(NodeInstance {
            lattice_pos: pos,
            world_pos,
            color,
            activation,
            octaves,
            age,
            seed,
            outlined,
            hovered: hovered == Some(pos),
            scale: depth_scale(world_pos.distance(eye), camera.distance),
            on_home: pos.sevens == view.center_sevens,
            cents: node_pc.to_cents(),
        });
    }

    let edges = if view.show_chord_edges { derive_edges(&nodes) } else { Vec::new() };
    let grid = derive_grid(view, &nodes);

    // Core/outer geometry policy: the core is a plain radius the shader
    // reads (0 = off), with solidity riding alongside; the outer band is
    // sanitized here so the shader can trust outer > inner whatever the two
    // bars hold.
    let core_radius = view.core_radius.clamp(0.0, 0.9);
    let core_solidity = view.core_solidity.clamp(0.0, 1.0);
    let outer_inner = view.outer_inner.clamp(0.0, 0.9);
    let outer_outer = view.outer_outer.clamp(outer_inner + 0.05, 1.0);
    let outer_solidity = view.outer_solidity.clamp(0.0, 1.0);
    let outer_backdrop = view.outer_backdrop.clamp(0.0, 1.0);

    Scene {
        nodes,
        camera,
        time: (now % 3600.0) as f32,
        node_radius: view.spacing * NODE_RADIUS_FACTOR,
        outer_style: view.outer_style,
        node_style: view.node_style,
        core_radius,
        core_solidity,
        outer_inner,
        outer_outer,
        outer_backdrop,
        outer_solidity,
        idle_marker: view.idle_marker,
        idle_radius: view.idle_radius.clamp(0.0, 0.9),
        edges,
        grid,
        grid_thickness: view.grid_thickness.clamp(0.0, 8.0),
        dot_ramp: pitch_ramp_lut(),
        darkest_pitch: frame.darkest_pitch,
        brightest_pitch: frame.brightest_pitch,
        render_scale: view.render_scale,
        bloom_strength: view.bloom_strength,
    }
}

/// Depth-cue strength: the exponent on (focus distance / node distance)
/// that sets a node's size multiplier. 0 would disable the cue (plain
/// perspective); 1 roughly doubles perspective's own shrink-with-distance.
const DEPTH_SCALE_EXPONENT: f32 = 0.8;
/// Clamp on the multiplier so nodes stay recognizable when the camera
/// gets very close to (or very far from) part of the lattice.
const DEPTH_SCALE_RANGE: (f32, f32) = (0.4, 2.0);

/// Depth-cue size multiplier for a node `dist` from the eye, with the
/// camera focused (eye-to-target) at `focus`: 1 at the focus distance, so
/// the lattice's overall look is unchanged where the user is looking;
/// larger when nearer, smaller when farther. Perspective alone shrinks a
/// distant node too subtly for depth to read at lattice scale — this
/// exaggerates it.
fn depth_scale(dist: f32, focus: f32) -> f32 {
    (focus / dist.max(0.01))
        .powf(DEPTH_SCALE_EXPONENT)
        .clamp(DEPTH_SCALE_RANGE.0, DEPTH_SCALE_RANGE.1)
}

/// Line opacity of a grid segment whose two endpoint notes both sound.
const GRID_LIT_OPACITY: f32 = 0.85;

/// The faint background grid: idle positions draw no disc, so these
/// segments carry the lattice's structure instead, inset at both ends so
/// each node position keeps a clear circular gap. Only the home (center)
/// sheet draws an idle grid; other sheets' lines light when both
/// endpoints sound, and the dashed sevens-axis links light as the chain
/// from any sounding off-sheet note down to the home sheet. Lit segments
/// take the sounding notes' color and fade with their envelope.
fn derive_grid(view: &ViewConfig, nodes: &[NodeInstance]) -> Vec<EdgeInstance> {
    let inset = view.spacing * NODE_RADIUS_FACTOR * view.grid_inset.max(0.0);
    let base = Vec4::from_array(view.grid_color);
    // Half-cell offset: nodes sit at world (fives, threes) integers, so
    // shifting the in-plane lines by half a cell in both puts them at the
    // half-integers — i.e. the notes land in the middle of each square
    // rather than on its corners. Sevens links keep their true endpoints.
    let half_offset = if view.grid_half_offset {
        Vec3::new(0.5, 0.5, 0.0) * view.spacing
    } else {
        Vec3::ZERO
    };
    // Presized: this rebuilds every frame, and collect() would otherwise
    // rehash several times as it grows past its default capacity.
    let mut index: std::collections::HashMap<LatticePos, &NodeInstance> =
        std::collections::HashMap::with_capacity(nodes.len());
    index.extend(nodes.iter().map(|n| (n.lattice_pos, n)));
    let mut grid = Vec::new();
    for node in nodes {
        let p = node.lattice_pos;
        // +1 steps only, so each undirected pair appears once; positions
        // outside the window simply miss the index.
        for (axis, step) in [
            LatticePos::new(p.threes + 1, p.fives, p.sevens),
            LatticePos::new(p.threes, p.fives + 1, p.sevens),
            LatticePos::new(p.threes, p.fives, p.sevens + 1),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(neighbor) = index.get(&step) else {
                continue;
            };
            let along_sevens = axis == 2;
            // Only the home (center) sheet draws an idle grid; other
            // sheets' lines and the links between sheets stay invisible
            // until the music lights them. Links render dashed.
            let on_home = !along_sevens && p.sevens == view.center_sevens;
            let idle = if on_home { base.w } else { 0.0 };
            // The sevens links are always dashed: that dash is what tells a
            // depth link from an in-sheet line, not a style choice.
            let dashed = along_sevens || view.grid_dashed;

            // Both endpoints sounding lights any segment.
            let mut lit = node.activation.min(neighbor.activation);
            let mut lit_color = (node.color + neighbor.color) * 0.5;

            // A sevens link also lights as part of the chain from any
            // sounding node beyond it (away from the home sheet) down to
            // the home sheet, so an off-sheet note always hangs from a
            // visible chain even while the notes under it are silent.
            if along_sevens {
                let (lo, hi) = if p.sevens >= view.center_sevens {
                    (p.sevens + 1, view.center_sevens + view.extent_sevens)
                } else {
                    (view.center_sevens - view.extent_sevens, p.sevens)
                };
                for s in lo..=hi {
                    if let Some(n) = index.get(&LatticePos::new(p.threes, p.fives, s)) {
                        if n.activation > lit {
                            lit = n.activation;
                            lit_color = n.color;
                        }
                    }
                }
            }

            // Fully invisible: skip the instance instead of shipping a
            // discarded quad.
            if idle <= 0.0 && lit <= 0.0 {
                continue;
            }
            let dir = (neighbor.world_pos - node.world_pos).normalize_or_zero();
            let shift = if along_sevens { Vec3::ZERO } else { half_offset };
            grid.push(EdgeInstance {
                a: node.world_pos + shift + dir * inset,
                b: neighbor.world_pos + shift - dir * inset,
                color: base.lerp(lit_color, lit),
                strength: idle + (GRID_LIT_OPACITY - idle) * lit,
                dashed,
            });
        }
    }
    grid
}

/// Stable per-node animation seed for the field styles: a hash of the
/// lattice position folded into the same small range as the per-note
/// seed. A node's gas pattern becomes part of its identity — the same
/// every time it lights, decorrelated from its neighbors'.
fn node_seed(pos: LatticePos) -> f32 {
    let h = pos
        .threes
        .wrapping_mul(731)
        .wrapping_add(pos.fives.wrapping_mul(2683))
        .wrapping_add(pos.sevens.wrapping_mul(9461));
    (h as f32 * 0.618_034).rem_euclid(256.0)
}

/// Chord edges: every pair of active, lattice-adjacent nodes gets a beam.
/// O(active²), and active counts are tiny.
fn derive_edges(nodes: &[NodeInstance]) -> Vec<EdgeInstance> {
    let active: Vec<&NodeInstance> = nodes.iter().filter(|n| n.activation > 0.0).collect();
    let mut edges = Vec::new();
    for (i, a) in active.iter().enumerate() {
        for b in &active[i + 1..] {
            if a.lattice_pos.is_adjacent(b.lattice_pos) {
                edges.push(EdgeInstance {
                    a: a.world_pos,
                    b: b.world_pos,
                    color: (a.color + b.color) * 0.5,
                    strength: a.activation.min(b.activation),
                    dashed: false,
                });
            }
        }
    }
    edges
}

/// Cached projection for one (camera, viewport) pair: build once, then
/// project many points (labels, picking) without rebuilding the
/// view-projection matrix per node.
pub struct Projector {
    view_proj: Mat4,
    viewport_px: Vec2,
}

impl Projector {
    /// Project a world position into viewport pixels (origin top-left).
    /// `None` when behind the camera.
    pub fn project(&self, world: Vec3) -> Option<Vec2> {
        let clip = self.view_proj * world.extend(1.0);
        // Behind the camera (or nearer than the near plane): perspective
        // flips w negative there; orthographic keeps w at 1 and instead
        // sends clip z below the near plane's 0. Test both so the check
        // holds under either projection.
        if clip.w <= 0.0 || clip.z < 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x * 0.5 + 0.5) * self.viewport_px.x,
            (1.0 - (ndc.y * 0.5 + 0.5)) * self.viewport_px.y,
        ))
    }
}

impl Scene {
    pub fn projector(&self, viewport_px: Vec2) -> Projector {
        Projector {
            view_proj: self.camera.view_proj(viewport_px.x / viewport_px.y.max(1.0)),
            viewport_px,
        }
    }

    /// Convenience for one-off projections; per-node loops should reuse a
    /// [`Scene::projector`].
    pub fn project(&self, viewport_px: Vec2, world: Vec3) -> Option<Vec2> {
        self.projector(viewport_px).project(world)
    }

    /// CPU picking: the node whose screen projection is nearest the pointer,
    /// within `max_px`. Every pane that wants "hover a pitch class" uses
    /// this and writes the result to the shared UI state.
    ///
    /// Only [visible](NodeInstance::is_visible) nodes are pickable, so the
    /// pointer can't pull a pitch readout out of an off-sheet node that
    /// draws nothing.
    pub fn pick(&self, viewport_px: Vec2, pointer_px: Vec2, max_px: f32) -> Option<LatticePos> {
        let projector = self.projector(viewport_px);
        let mut best: Option<(f32, LatticePos)> = None;
        for node in &self.nodes {
            if !node.is_visible() {
                continue;
            }
            let Some(px) = projector.project(node.world_pos) else {
                continue;
            };
            let d = px.distance(pointer_px);
            if d <= max_px && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, node.lattice_pos));
            }
        }
        best.map(|(_, pos)| pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_core::{NoteEvent, NoteEventKind, PitchClass};

    fn scene_of(
        tracker: &NoteTracker,
        tuning: &Tuning,
        view: &ViewConfig,
        frame: &FrameParams,
        now: f64,
    ) -> Scene {
        derive_scene(tracker, tuning, view, frame, Camera::default(), None, now)
    }

    fn origin_node(scene: &Scene) -> &NodeInstance {
        scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == LatticePos::ORIGIN)
            .unwrap()
    }

    #[test]
    fn pitch_colored_channels_vary_with_pitch() {
        let low = channel_color(9, 24.0, 24.0, 108.0);
        let high = channel_color(9, 108.0, 24.0, 108.0);
        assert_ne!(low, high);
        // Brightest pitch should be, well, brighter.
        assert!(high.truncate().length() > low.truncate().length());
    }

    #[test]
    fn dot_ramp_lut_reproduces_the_pitch_gradient() {
        // The dots octave style tints each dot by sampling `pitch_ramp_lut`
        // the way the shader does (linear interp across DOT_RAMP_N entries).
        // Reconstructing that here must land on the disc's gradient color for
        // the same pitch, so a dot is the color of the disc its pitch lights.
        let lut = pitch_ramp_lut();
        let (dark, bright) = (24.0f32, 108.0f32);
        for pitch in [24.0f32, 36.0, 54.0, 60.0, 72.0, 96.0, 108.0] {
            let t = ((pitch - dark) / (bright - dark)).clamp(0.0, 1.0);
            let f = t * (DOT_RAMP_N - 1) as f32;
            let i0 = f.floor() as usize;
            let i1 = (i0 + 1).min(DOT_RAMP_N - 1);
            let lut_color = lut[i0].lerp(lut[i1], f - f.floor());
            // Same pitch through the disc path (channel 9 is pitch-gradient).
            let disc = channel_color(9, pitch, dark, bright);
            assert!(
                (lut_color - disc).truncate().length() < 0.05,
                "pitch {pitch}: lut {lut_color:?} vs disc {disc:?}"
            );
        }
    }

    #[test]
    fn octaves_fade_independently() {
        // Hold C4, tap-and-release C5: the octave-5 indicator must decay on
        // its own envelope even though the node stays fully active.
        let mut tracker = NoteTracker::new();
        for (note, kind) in [
            (60, NoteEventKind::On { velocity: 1.0 }), // C4 held
            (72, NoteEventKind::On { velocity: 1.0 }), // C5 tapped...
        ] {
            tracker.handle_event(NoteEvent { time: 0.0, channel: 0, note, kind });
        }
        tracker.handle_event(NoteEvent {
            time: 0.1,
            channel: 0,
            note: 72,
            kind: NoteEventKind::Off, // ...and released
        });

        // Half a pitch_class_fade_time after the release.
        let frame = FrameParams { pitch_class_fade_time: 1.0, ..FrameParams::default() };
        let scene =
            scene_of(&tracker, &Tuning::default(), &ViewConfig::default(), &frame, 0.6);
        let origin = origin_node(&scene);
        assert_eq!(origin.activation, 1.0, "node stays lit by the held C4");
        assert_eq!(origin.octaves[4], 1.0, "held octave at full");
        assert!(
            origin.octaves[5] > 0.0 && origin.octaves[5] < 0.75,
            "released octave mid-fade, got {}",
            origin.octaves[5]
        );
    }

    #[test]
    fn octave_fade_time_is_independent_of_note_fade() {
        // Note fade short, octave fade long: after the note highlight ends,
        // the disc goes idle but the octave indicator is still fading.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Off,
        });
        let frame = FrameParams {
            pitch_class_fade_time: 0.2,
            octave_fade_time: 2.0,
            ..FrameParams::default()
        };
        // Prune with the longer of the two, as root_ui does.
        tracker.prune(1.0, frame.pitch_class_fade_time.max(frame.octave_fade_time));
        let scene =
            scene_of(&tracker, &Tuning::default(), &ViewConfig::default(), &frame, 1.0);
        let origin = origin_node(&scene);
        assert_eq!(origin.activation, 0.0, "pitch class fade has ended");
        assert!(
            origin.octaves[4] > 0.0 && origin.octaves[4] < 0.75,
            "octave still mid-fade, got {}",
            origin.octaves[4]
        );
    }

    #[test]
    fn age_and_seed_derive_from_the_note_on_time() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 10.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        // Steady seeds each node from its note-on time; field styles seed
        // from position instead, so pin the style this test is about.
        let view = ViewConfig { node_style: NodeStyle::Steady, ..ViewConfig::default() };
        let scene = scene_of(
            &tracker,
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            12.5,
        );
        let origin = origin_node(&scene);
        assert!((origin.age - 2.5).abs() < 1e-6);
        assert!((origin.seed - 10.0).abs() < 1e-6);
        // Idle nodes carry neutral animation inputs.
        let idle = scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == LatticePos::new(1, 1, 0))
            .unwrap();
        assert_eq!((idle.age, idle.seed), (0.0, 0.0));
    }

    #[test]
    fn window_center_pans_which_nodes_display() {
        let view = ViewConfig {
            center_threes: 5,
            extent_threes: 1,
            extent_fives: 0,
            extent_sevens: 0,
            ..ViewConfig::default()
        };
        let positions: Vec<_> = view.visible_positions().collect();
        assert_eq!(
            positions,
            vec![
                LatticePos::new(4, 0, 0),
                LatticePos::new(5, 0, 0),
                LatticePos::new(6, 0, 0)
            ]
        );

        // The center node renders at the world origin.
        let tracker = NoteTracker::new();
        let scene =
            scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 0.0);
        let center_node = scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == LatticePos::new(5, 0, 0))
            .unwrap();
        assert_eq!(center_node.world_pos, Vec3::ZERO);
    }

    #[test]
    fn chord_edges_connect_adjacent_active_nodes() {
        // A deliberately small window (±3/±3): just intonation keeps pitch
        // classes unique at that size, so the edge count is exact. Wider
        // windows bring in schisma near-duplicates — e.g. (8, 1, 0) sits
        // ~2¢ from C, within this test's 5¢ tolerance — which light up and
        // add parallel edges (correct behavior, but noisy to pin; same
        // reason 12-TET's enharmonic duplicates are avoided here).
        // C and G (a fifth apart, one step on the prime-3 axis) held
        // together with a wide-enough tolerance: exactly one edge.
        let tuning = Tuning { tolerance: lattice_core::tuning::microcents(5.0), ..Tuning::just() };
        let mut tracker = NoteTracker::new();
        for note in [60u8, 67] {
            tracker.handle_event(NoteEvent {
                time: 0.0,
                channel: 0,
                note,
                kind: NoteEventKind::On { velocity: 1.0 },
            });
        }
        let view = ViewConfig {
            show_chord_edges: true,
            extent_threes: 3,
            extent_fives: 3,
            ..ViewConfig::default()
        };
        let scene = scene_of(&tracker, &tuning, &view, &FrameParams::default(), 0.0);
        assert_eq!(scene.edges.len(), 1);
        assert_eq!(scene.edges[0].strength, 1.0);

        // C and F# (nothing within a step and 5 cents): no edge.
        let mut tracker = NoteTracker::new();
        for note in [60u8, 66] {
            tracker.handle_event(NoteEvent {
                time: 0.0,
                channel: 0,
                note,
                kind: NoteEventKind::On { velocity: 1.0 },
            });
        }
        let scene = scene_of(&tracker, &tuning, &view, &FrameParams::default(), 0.0);
        assert_eq!(scene.edges.len(), 0);
    }

    #[test]
    fn grid_segments_connect_neighbors_but_leave_node_gaps() {
        // A 3×3 window: 2·3 horizontal + 3·2 vertical inter-neighbor
        // segments, none along the unused sevens axis.
        let view = ViewConfig {
            extent_threes: 1,
            extent_fives: 1,
            extent_sevens: 0,
            ..ViewConfig::default()
        };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            0.0,
        );
        assert_eq!(scene.grid.len(), 12);
        for seg in &scene.grid {
            // Inset at both ends: shorter than the node spacing...
            let len = seg.a.distance(seg.b);
            assert!(len < view.spacing * 0.99, "segment not inset, len {len}");
            // ...and clear of every disc (visual radius ~0.9 × node_radius),
            // so the gap fully contains the circle a played note draws.
            for node in &scene.nodes {
                for p in [seg.a, seg.b] {
                    assert!(
                        p.distance(node.world_pos) > scene.node_radius * 0.9,
                        "segment endpoint {p:?} inside the disc at {:?}",
                        node.world_pos
                    );
                }
            }
        }

        // Panning the window keeps the grid attached to the visible nodes
        // (both are derived in centered world space).
        let panned = ViewConfig { center_threes: 3, ..view };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &panned,
            &FrameParams::default(),
            0.0,
        );
        assert_eq!(scene.grid.len(), 12);
        let max_node = scene
            .nodes
            .iter()
            .map(|n| n.world_pos.length())
            .fold(0.0f32, f32::max);
        for seg in &scene.grid {
            assert!(seg.a.length() <= max_node && seg.b.length() <= max_node);
        }
    }

    /// A 7x7 window one sevens step deep, so sevens links are in play too.
    /// Wide enough that a held C also lands on an off-sheet node — under
    /// 12-TET that first happens at (2, -3, 1) — which is what lights the
    /// links; a ±1 window has no such node and would show none at all.
    fn grid_view() -> ViewConfig {
        ViewConfig {
            extent_threes: 3,
            extent_fives: 3,
            extent_sevens: 1,
            ..ViewConfig::default()
        }
    }

    fn grid_of(view: &ViewConfig) -> Vec<EdgeInstance> {
        grid_of_with(view, &NoteTracker::new())
    }

    fn grid_of_with(view: &ViewConfig, tracker: &NoteTracker) -> Vec<EdgeInstance> {
        scene_of(tracker, &Tuning::default(), view, &FrameParams::default(), 0.0).grid
    }

    /// A held note, so the off-sheet sevens links light up and appear in
    /// the grid at all (idle ones are skipped as fully invisible).
    fn sounding() -> NoteTracker {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        tracker
    }

    #[test]
    fn grid_inset_sets_how_far_lines_stop_short_of_a_node() {
        // The inset is the "line length" knob: at 0 a segment spans the
        // full node spacing, and raising it eats the line from both ends.
        let flush = grid_of(&ViewConfig { grid_inset: 0.0, ..grid_view() });
        let spacing = grid_view().spacing;
        for seg in &flush {
            assert!(
                (seg.a.distance(seg.b) - spacing).abs() < 1e-5,
                "inset 0 should span the whole spacing, got {}",
                seg.a.distance(seg.b)
            );
        }

        let mut lengths = vec![];
        for inset in [0.0f32, 0.5, 1.05, 2.0] {
            let grid = grid_of(&ViewConfig { grid_inset: inset, ..grid_view() });
            lengths.push(grid[0].a.distance(grid[0].b));
        }
        for pair in lengths.windows(2) {
            assert!(pair[1] < pair[0], "more inset must mean shorter lines: {lengths:?}");
        }
    }

    #[test]
    fn grid_half_offset_moves_in_plane_lines_between_the_nodes() {
        // Nodes sit at whole multiples of the spacing; offsetting by half a
        // cell in both in-plane axes puts every in-plane line on the
        // half-multiples, so the notes land in the middle of each square.
        // The sevens links must NOT move: they connect real notes across
        // sheets, and shifting them off their nodes would break that.
        let view = grid_view();
        let tracker = sounding();
        let plain = grid_of_with(&view, &tracker);
        let offset =
            grid_of_with(&ViewConfig { grid_half_offset: true, ..view.clone() }, &tracker);
        assert_eq!(plain.len(), offset.len(), "the offset only moves lines, never adds any");

        let half = Vec3::new(0.5, 0.5, 0.0) * view.spacing;
        let mut moved = 0;
        let mut stayed = 0;
        for (before, after) in plain.iter().zip(&offset) {
            // Sevens links run along world z; in-plane lines don't.
            let along_sevens = (before.b.z - before.a.z).abs() > 1e-5;
            if along_sevens {
                assert_eq!(after.a, before.a, "a sevens link must stay put");
                assert_eq!(after.b, before.b);
                stayed += 1;
            } else {
                assert!((after.a - (before.a + half)).length() < 1e-5, "in-plane line must shift");
                assert!((after.b - (before.b + half)).length() < 1e-5);
                moved += 1;
            }
        }
        assert!(moved > 0 && stayed > 0, "want both kinds present: {moved} moved, {stayed} stayed");
    }

    #[test]
    fn grid_color_and_dashes_come_from_the_view() {
        // The color (and its alpha, the idle line opacity) is a view
        // setting now; it used to be readable only from the skin.
        let tinted = [0.9f32, 0.1, 0.4, 0.25];
        let grid = grid_of(&ViewConfig { grid_color: tinted, ..grid_view() });
        let unlit = grid
            .iter()
            .find(|s| s.strength > 0.0)
            .expect("the home sheet draws an idle grid");
        assert_eq!(unlit.color, Vec4::from_array(tinted));
        assert_eq!(unlit.strength, tinted[3], "alpha is the idle line opacity");

        // Dashes: off by default for in-plane lines, on for sevens links
        // either way (that dash marks a depth link, it isn't a style).
        let tracker = sounding();
        let plain = grid_of_with(&grid_view(), &tracker);
        let dashed =
            grid_of_with(&ViewConfig { grid_dashed: true, ..grid_view() }, &tracker);
        assert!(
            plain.iter().any(|s| (s.b.z - s.a.z).abs() > 1e-5),
            "want some sevens links in the mix"
        );
        for (before, after) in plain.iter().zip(&dashed) {
            let along_sevens = (before.b.z - before.a.z).abs() > 1e-5;
            assert_eq!(before.dashed, along_sevens, "only links dash by default");
            assert!(after.dashed, "every line dashes when the style is on");
        }
    }

    #[test]
    fn home_sheet_nodes_are_flagged_for_the_blank_ring() {
        // Follows the panned window center, not sevens == 0.
        let view = ViewConfig {
            extent_threes: 0,
            extent_fives: 0,
            extent_sevens: 1,
            center_sevens: 2,
            ..ViewConfig::default()
        };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            0.0,
        );
        for n in &scene.nodes {
            assert_eq!(n.on_home, n.lattice_pos.sevens == 2, "{:?}", n.lattice_pos);
        }
    }

    #[test]
    fn off_sheet_grid_appears_only_where_the_music_reaches() {
        // A window two sevens layers deep above/below the center, so the
        // chain rule has an intermediate link to prove itself on.
        let view = ViewConfig {
            extent_threes: 1,
            extent_fives: 0,
            extent_sevens: 2,
            ..ViewConfig::default()
        };
        let is_link = |s: &EdgeInstance| (s.b.z - s.a.z).abs() > 0.25;
        let off_home = |s: &EdgeInstance| is_link(s) || s.a.z.abs() > 0.5;

        // Idle: only the home sheet's solid lines exist.
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            0.0,
        );
        assert!(!scene.grid.is_empty());
        assert!(scene.grid.iter().all(|s| !off_home(s) && !s.dashed && s.strength > 0.0));

        // Hold the note two sevens steps up from C (12-TET default:
        // 2 × 1000¢ → pitch class 800¢ = G#/Ab, MIDI 68). It lights node
        // (0,0,2) only, yet BOTH links of the chain down to the home
        // sheet must display, dashed, in that note's color — the nodes
        // under it are silent.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 68,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let scene =
            scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 0.0);
        let column_links: Vec<&EdgeInstance> = scene
            .grid
            .iter()
            .filter(|s| is_link(s) && s.a.x.abs() < 0.01 && s.a.y.abs() < 0.01)
            .collect();
        // The two links spanning 0->1 and 1->2; nothing below the sheet.
        assert_eq!(column_links.len(), 2, "{column_links:?}");
        for link in &column_links {
            assert!(link.a.z > -0.5 && link.dashed && link.strength > 0.5, "{link:?}");
        }
        // No off-sheet IN-SHEET lines appeared: the played node's sheet
        // neighbors are silent, so only the chain and home sheet render.
        assert!(scene
            .grid
            .iter()
            .all(|s| is_link(s) || s.a.z.abs() < 0.5));
    }

    #[test]
    fn core_and_outer_geometry_are_sanitized_into_the_scene() {
        // Bars dragged into a crossed/degenerate combination: the scene
        // must still hand the shader a visible band (outer ahead of inner).
        // A radius of 0 turns the core off (passes through as 0).
        let view = ViewConfig {
            core_radius: 0.0,
            outer_style: OuterStyle::Slices,
            outer_inner: 0.8,
            outer_outer: 0.3,
            ..ViewConfig::default()
        };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            0.0,
        );
        assert_eq!(scene.outer_style, OuterStyle::Slices);
        assert_eq!(scene.core_radius, 0.0, "radius 0 = core off");
        assert_eq!(scene.outer_inner, 0.8);
        assert!(scene.outer_outer > scene.outer_inner);

        // Core on: the radius passes through and solidity rides alongside,
        // both clamped to range.
        let view = ViewConfig {
            core_radius: 0.3,
            core_solidity: 0.25,
            outer_inner: 0.0,
            outer_outer: 0.5,
            ..view
        };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            0.0,
        );
        assert_eq!(scene.core_radius, 0.3);
        assert_eq!(scene.core_solidity, 0.25);
        assert_eq!((scene.outer_inner, scene.outer_outer), (0.0, 0.5));

        // Both solidities are clamped into 0..1 before they reach the shader.
        let view = ViewConfig { core_solidity: 4.0, outer_solidity: -1.0, ..view };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            0.0,
        );
        assert_eq!(scene.core_solidity, 1.0);
        assert_eq!(scene.outer_solidity, 0.0);
    }

    #[test]
    fn legacy_core_modes_fold_onto_radius_and_solidity() {
        // The pre-radius-off modes collapse onto core_radius (0 = off) +
        // solidity: None -> radius 0, the solid Orb -> solidity 1, the
        // glow-only mode -> solidity 0.
        let mut none = ViewConfig { core_style: CoreStyle::None, ..ViewConfig::default() };
        none.migrate_legacy();
        assert_eq!(none.core_radius, 0.0, "None folds to radius 0 (off)");

        let mut orb = ViewConfig { core_style: CoreStyle::Orb, ..ViewConfig::default() };
        orb.migrate_legacy();
        assert_eq!(orb.core_solidity, 1.0);
        assert!(orb.core_radius > 0.0, "orb stays on");

        let mut glow = ViewConfig { core_style: CoreStyle::Glow, ..ViewConfig::default() };
        glow.migrate_legacy();
        assert_eq!(glow.core_solidity, 0.0);
        assert!(glow.core_radius > 0.0, "glow stays on");
    }

    #[test]
    fn legacy_node_body_folds_into_core_and_outer() {
        // Blobs from the one-build NodeBody experiment: an octave-only body
        // becomes the core glow (solidity 0, the old core-off under-glow) +
        // the matching outer style with the backdrop on; Beads maps to Dots
        // (whose backdrop hoop IS the beads look). Disc leaves defaults alone.
        for (body, outer) in [
            (LegacyNodeBody::Slices, OuterStyle::Slices),
            (LegacyNodeBody::Rings, OuterStyle::Rings),
            (LegacyNodeBody::Beads, OuterStyle::Dots),
        ] {
            let mut view = ViewConfig { node_body: body, ..ViewConfig::default() };
            view.migrate_legacy();
            assert_eq!(view.core_solidity, 0.0, "{body:?}");
            assert!(view.core_radius > 0.0, "{body:?} still on");
            assert_eq!(view.outer_style, outer, "{body:?}");
            assert_eq!(view.outer_backdrop, 1.0, "{body:?}");
            assert_eq!(view.node_body, LegacyNodeBody::Disc, "shim consumed");
        }

        let mut view = ViewConfig { node_body: LegacyNodeBody::Disc, ..ViewConfig::default() };
        view.migrate_legacy();
        assert_eq!(view.core_solidity, 1.0);
        assert_eq!(view.outer_style, ViewConfig::default().outer_style);
    }

    #[test]
    fn depth_scale_exaggerates_proximity() {
        // Neutral at the focus distance, monotonic on either side, clamped
        // at the extremes.
        assert!((depth_scale(12.0, 12.0) - 1.0).abs() < 1e-6);
        assert!(depth_scale(6.0, 12.0) > 1.0);
        assert!(depth_scale(24.0, 12.0) < 1.0);
        assert_eq!(depth_scale(0.001, 12.0), DEPTH_SCALE_RANGE.1);
        assert_eq!(depth_scale(1e6, 12.0), DEPTH_SCALE_RANGE.0);

        // And the scene wires it in: the node nearest the eye renders
        // larger than the farthest one.
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        let eye = scene.camera.eye();
        let dist = |n: &&NodeInstance| n.world_pos.distance(eye);
        let near = scene.nodes.iter().min_by(|a, b| dist(a).total_cmp(&dist(b))).unwrap();
        let far = scene.nodes.iter().max_by(|a, b| dist(a).total_cmp(&dist(b))).unwrap();
        assert!(
            near.scale > far.scale,
            "near {} vs far {}",
            near.scale,
            far.scale
        );
    }

    #[test]
    fn grid_lights_between_played_neighbors() {
        // C and G held (one step on the threes axis; same window/tuning
        // rationale as chord_edges_connect_adjacent_active_nodes): the
        // segment between them takes the lit opacity and the notes' blended
        // color; a segment with only one sounding endpoint stays at the
        // faint base look.
        let tuning = Tuning { tolerance: lattice_core::tuning::microcents(5.0), ..Tuning::just() };
        let mut tracker = NoteTracker::new();
        for note in [60u8, 67] {
            tracker.handle_event(NoteEvent {
                time: 0.0,
                channel: 0,
                note,
                kind: NoteEventKind::On { velocity: 1.0 },
            });
        }
        let view = ViewConfig {
            extent_threes: 3,
            extent_fives: 3,
            ..ViewConfig::default()
        };
        let scene = scene_of(&tracker, &tuning, &view, &FrameParams::default(), 0.0);
        let base = skin::active_skin().grid_line;
        let segment_at = |mid: Vec3| {
            scene
                .grid
                .iter()
                .find(|s| ((s.a + s.b) * 0.5).distance(mid) < 1e-4)
                .unwrap()
        };

        // C sits at the origin, G one step up the threes (world y) axis.
        let lit = segment_at(Vec3::new(0.0, 0.5, 0.0));
        assert!(lit.strength > base.w, "lit opacity, got {}", lit.strength);
        assert!(
            (lit.color - base).truncate().length() > 0.1,
            "lit segment tinted by the notes, got {:?}",
            lit.color
        );

        // F–C below: C sounds but F doesn't, so the segment stays faint.
        let unlit = segment_at(Vec3::new(0.0, -0.5, 0.0));
        assert_eq!(unlit.strength, base.w);
        assert_eq!(unlit.color, base);
    }

    #[test]
    fn channel_14_voices_render_outlined() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 14,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let scene = scene_of(
            &tracker,
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        assert!(origin_node(&scene).outlined);
    }

    #[test]
    fn held_note_lights_matching_nodes() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60, // C4: pitch class 0, octave 4
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let tuning = Tuning::default(); // 12-TET: origin node matches C exactly
        // Sampled after OCTAVE_ATTACK_TIME: the octave indicator eases in,
        // so at the note-on instant itself it is still at zero.
        let scene = scene_of(
            &tracker,
            &tuning,
            &ViewConfig::default(),
            &FrameParams::default(),
            0.5,
        );
        let origin = origin_node(&scene);
        assert_eq!(origin.activation, 1.0);
        assert_eq!(origin.octaves[4], 1.0);
    }

    #[test]
    fn camera_target_projects_to_viewport_center() {
        let camera = Camera::default();
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        let viewport = Vec2::new(800.0, 600.0);
        let p = scene.project(viewport, camera.target).unwrap();
        assert!((p.x - 400.0).abs() < 0.5, "x = {}", p.x);
        assert!((p.y - 300.0).abs() < 0.5, "y = {}", p.y);
    }

    #[test]
    fn points_behind_the_camera_do_not_project() {
        for projection in [
            Projection::Perspective,
            Projection::Orthographic,
            Projection::Cabinet,
        ] {
            let camera = Camera { projection, ..Camera::default() };
            let mut scene = scene_of(
                &NoteTracker::new(),
                &Tuning::default(),
                &ViewConfig::default(),
                &FrameParams::default(),
                0.0,
            );
            scene.camera = camera;
            // Continue from the target through the eye and beyond it.
            let behind = camera.eye() + (camera.eye() - camera.target);
            assert_eq!(
                scene.project(Vec2::new(800.0, 600.0), behind),
                None,
                "{projection:?}"
            );
        }
    }

    #[test]
    fn cabinet_faces_the_sheet_and_shears_sevens_uniformly() {
        let viewport = Vec2::new(800.0, 600.0);
        // Orbit angles are ignored: cabinet always faces the sheet. Pin the
        // shear scale to 0.5 so the "half scale" checks below hold whatever
        // the default is.
        let camera = Camera {
            projection: Projection::Cabinet,
            yaw: 1.0,
            pitch: -0.7,
            cabinet_scale: 0.5,
            ..Camera::default()
        };
        assert_eq!(camera.eye(), Vec3::new(0.0, 0.0, camera.distance));

        let mut s = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        s.camera = camera;
        let px = |w: Vec3| s.project(viewport, w).unwrap();

        // Target centered; front-plane steps map to pure screen axes
        // (the sheet renders undistorted).
        let origin = px(Vec3::ZERO);
        assert!((origin - Vec2::new(400.0, 300.0)).length() < 0.5, "{origin:?}");
        let dx = px(Vec3::X) - origin;
        assert!(dx.x > 1.0 && dx.y.abs() < 1e-3, "{dx:?}");
        let dy = px(Vec3::Y) - origin;
        assert!(dy.y < -1.0 && dy.x.abs() < 1e-3, "{dy:?}"); // screen y is down

        // A +sevens step (toward the viewer) is the same up-right arrow
        // anywhere on the sheet, at half scale split evenly over x/y.
        let dz = px(Vec3::Z) - origin;
        let dz_elsewhere = px(Vec3::new(3.0, -2.0, 1.0)) - px(Vec3::new(3.0, -2.0, 0.0));
        assert!(dz.distance(dz_elsewhere) < 1e-3, "{dz:?} vs {dz_elsewhere:?}");
        let k = 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((dz.x - dx.x * k).abs() < 0.1, "{dz:?} vs {dx:?}");
        assert!((dz.y - dy.y * k).abs() < 0.1, "{dz:?} vs {dy:?}");

        // The knobs steer the arrow: angle 0 at full (cavalier) scale
        // shears purely horizontally, one front-plane step long.
        s.camera.cabinet_angle = 0.0;
        s.camera.cabinet_scale = 1.0;
        let dz = s.project(viewport, Vec3::Z).unwrap() - s.project(viewport, Vec3::ZERO).unwrap();
        assert!((dz.x - dx.x).abs() < 0.1 && dz.y.abs() < 1e-3, "{dz:?} vs {dx:?}");
    }

    #[test]
    fn orthographic_matches_perspective_at_the_focus_plane_and_is_uniform() {
        let viewport = Vec2::new(800.0, 600.0);
        let perspective = Camera { projection: Projection::Perspective, ..Camera::default() };
        let ortho = Camera { projection: Projection::Orthographic, ..perspective };
        let mut s = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );

        // The target projects to the viewport center in both projections.
        s.camera = ortho;
        let p = s.project(viewport, ortho.target).unwrap();
        assert!((p.x - 400.0).abs() < 0.5 && (p.y - 300.0).abs() < 0.5, "{p:?}");

        // Framing matches at the focus plane: a point one unit up (in view
        // space) from the target lands on the same pixel either way.
        let (_, up) = perspective.right_up();
        let in_plane = perspective.target + up;
        let ortho_px = s.project(viewport, in_plane).unwrap();
        s.camera = perspective;
        let persp_px = s.project(viewport, in_plane).unwrap();
        assert!(ortho_px.distance(persp_px) < 0.5, "{ortho_px:?} vs {persp_px:?}");

        // The property the projection exists for: equal world offsets give
        // equal pixel offsets at ANY depth. Step one unit right at the
        // focus plane and again two units toward the eye; perspective
        // renders the nearer step longer, orthographic identically.
        s.camera = ortho;
        let (right, _) = ortho.right_up();
        let toward_eye = (ortho.eye() - ortho.target).normalize() * 2.0;
        let d_focus = s.project(viewport, ortho.target + right).unwrap()
            - s.project(viewport, ortho.target).unwrap();
        let d_near = s.project(viewport, ortho.target + toward_eye + right).unwrap()
            - s.project(viewport, ortho.target + toward_eye).unwrap();
        assert!(d_focus.distance(d_near) < 1e-3, "{d_focus:?} vs {d_near:?}");
    }

    #[test]
    fn pick_selects_the_node_nearest_the_pointer() {
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        let viewport = Vec2::new(800.0, 600.0);
        // Pointer exactly on the projected origin node must pick it, not a
        // neighbor; a pointer far outside every node picks nothing.
        let origin_px = scene.project(viewport, Vec3::ZERO).unwrap();
        assert_eq!(scene.pick(viewport, origin_px, 24.0), Some(LatticePos::ORIGIN));
        assert_eq!(scene.pick(viewport, Vec2::new(-500.0, -500.0), 24.0), None);
    }

    #[test]
    fn idle_off_sheet_nodes_are_not_pickable() {
        // An idle node off the home sheet draws nothing, so hovering where
        // it would be must not hand back its pitch. Sounding makes it
        // visible, and pickable again.
        let view = ViewConfig::default();
        let tuning = Tuning::default();
        let viewport = Vec2::new(800.0, 600.0);

        let idle = scene_of(
            &NoteTracker::new(),
            &tuning,
            &view,
            &FrameParams::default(),
            0.0,
        );
        let off = *idle
            .nodes
            .iter()
            .find(|n| !n.on_home)
            .expect("default view spans more than the home sheet");
        assert_eq!(off.activation, 0.0);
        assert!(!off.is_visible());
        let off_px = idle.project(viewport, off.world_pos).unwrap();
        assert_ne!(
            idle.pick(viewport, off_px, 24.0),
            Some(off.lattice_pos),
            "idle off-sheet node should not be pickable"
        );

        // Same position, now sounding: play a note carrying its pitch class.
        let pc = tuning.pitch_class(off.lattice_pos);
        let note = (60u8..72)
            .find(|&n| tuning.matches(pc, PitchClass::from_midi_note(n)))
            .expect("some MIDI note lands on this node under 12-TET");
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let lit = scene_of(&tracker, &tuning, &view, &FrameParams::default(), 0.0);
        let lit_off = lit
            .nodes
            .iter()
            .find(|n| n.lattice_pos == off.lattice_pos)
            .unwrap();
        assert!(lit_off.activation > 0.0, "the note should light this node");
        assert!(lit_off.is_visible());
        assert_eq!(
            lit.pick(viewport, off_px, 24.0),
            Some(off.lattice_pos),
            "sounding off-sheet node should be pickable again"
        );
    }

    #[test]
    fn camera_right_up_is_orthonormal_to_the_view() {
        let camera = Camera::default();
        let (right, up) = camera.right_up();
        assert!((right.length() - 1.0).abs() < 1e-5);
        assert!((up.length() - 1.0).abs() < 1e-5);
        assert!(right.dot(up).abs() < 1e-5);
        let view_dir = (camera.target - camera.eye()).normalize();
        assert!(right.dot(view_dir).abs() < 1e-5);
        assert!(up.dot(view_dir).abs() < 1e-5);
    }

    #[test]
    fn camera_input_respects_clamps() {
        let mut camera = Camera::default();
        camera.orbit(Vec2::new(0.0, 10_000.0));
        assert_eq!(camera.pitch, Camera::PITCH_LIMIT);
        camera.orbit(Vec2::new(0.0, -100_000.0));
        assert_eq!(camera.pitch, -Camera::PITCH_LIMIT);
        camera.zoom(1e6);
        assert_eq!(camera.distance, Camera::MIN_DISTANCE);
        camera.zoom(-1e9);
        assert_eq!(camera.distance, Camera::MAX_DISTANCE);
        // Panning moves the target in the view plane, never toward the eye.
        let before = camera.eye() - camera.target;
        camera.pan(Vec2::new(40.0, -25.0));
        let after = camera.eye() - camera.target;
        assert!((before - after).length() < 1e-4);
    }

    #[test]
    fn zoom_by_scales_distance_and_clamps() {
        let mut camera = Camera::default();
        let start = camera.distance;
        // factor > 1 pulls the eye in (distance divides down)...
        camera.zoom_by(2.0);
        assert!((camera.distance - start / 2.0).abs() < 1e-4);
        // ...and factor < 1 pushes it back out.
        camera.zoom_by(0.5);
        assert!((camera.distance - start).abs() < 1e-4);
        // A huge factor clamps at the near limit; a tiny one at the far limit.
        camera.zoom_by(1e6);
        assert_eq!(camera.distance, Camera::MIN_DISTANCE);
        camera.zoom_by(1e-6);
        assert_eq!(camera.distance, Camera::MAX_DISTANCE);
        // Non-positive factors are ignored (no divide-by-zero or sign flip).
        let held = camera.distance;
        camera.zoom_by(0.0);
        camera.zoom_by(-3.0);
        assert_eq!(camera.distance, held);
    }

    #[test]
    fn visible_count_matches_visible_positions() {
        // `visible_count` is a `Vec::with_capacity` hint; it must equal the
        // number `visible_positions` actually enumerates, including the
        // degenerate cases where a non-positive extent collapses an axis to
        // empty.
        for &(t, f, s) in &[(0, 0, 0), (2, 1, 0), (3, 3, 3), (1, 0, 4), (-1, 2, 0)] {
            let view = ViewConfig {
                extent_threes: t,
                extent_fives: f,
                extent_sevens: s,
                ..ViewConfig::default()
            };
            assert_eq!(
                view.visible_count(),
                view.visible_positions().count(),
                "extents ({t}, {f}, {s})"
            );
        }
    }
}
