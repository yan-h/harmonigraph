//! [`ViewConfig`] — the persisted, non-automatable visual settings — its
//! serde defaults and legacy-blob migration, plus the per-frame
//! [`FrameParams`] mirror of the host-automatable appearance parameters.

use crate::skin;
use crate::style::{
    CoreStyle, HighlightExtremes, IdleMarker, LegacyNodeBody, NodeStyle, OuterStyle,
};
use crate::trail::TrailMark;
use lattice_core::{coords, LatticePos};

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
    /// silent octaves faintly behind the sounding sectors, in the note
    /// color, so the annulus completes and a lone octave still reads as a
    /// whole note. Inert while the layer is Off. Was implicitly tied to
    /// "core off", then its own on/off toggle.
    ///
    /// Now an opacity, 0..1: 0 is off (exactly the old `false`) and 1 is
    /// the old `true`'s strength, with everything between available so the
    /// backdrop can sit as far under the sounding glyphs as you like. It
    /// scales the shader's built-in ghost level rather than
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
    /// Padding inside the octave layer, in quad UV units: the constant
    /// gap between one octave sector and the next, AND the gap separating
    /// the melody/bass rings from the band. One number, because they read
    /// as one rhythm — a ring sitting closer to the band than the sectors
    /// sit to each other looks like a mistake.
    ///
    /// Was fixed at 0.12 (the sectors' gap; the rings used a narrower one
    /// of their own). 0 closes the sectors into a solid annulus and seats
    /// the rings right against it.
    #[serde(default = "default_outer_gap")]
    pub outer_gap: f32,
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
    // ---- Melody / bass highlight -----------------------------------------
    /// Which of the outer held notes to mark, so the melody and/or bass
    /// line reads at a glance out of a chord. The mark rides the OUTER EDGE
    /// of that note's octave indicator and nothing else — it never touches
    /// the core, whose color is the note's own. That also makes it the layer
    /// that survives a chord voiced within a single pitch class: every
    /// octave of one note lands on the same node, differing only by slot.
    #[serde(default)]
    pub highlight_extremes: HighlightExtremes,
    /// Opacity of the part of a mark ring that is cut off from the octave
    /// responsible for it, 0..1.
    ///
    /// Each ring is slit at that octave's two sector boundaries — the slit
    /// IS the gap between two octaves, continued outward — which leaves the
    /// stretch of ring belonging to the marked octave separated from the
    /// remainder of the circle. That stretch always draws at full strength;
    /// this fades everything else. 1 keeps the whole circle (the ring reads
    /// as a ring, merely broken); 0 leaves only the arc over the marked
    /// octave, which says WHICH octave loudly at the cost of the shape.
    ///
    /// A Gap of 0 leaves no slit, so there is nothing to separate and this
    /// has no effect.
    #[serde(default = "default_mark_unlinked")]
    pub mark_unlinked: f32,
    /// How thick each melody/bass ring is, in quad UV units — the same
    /// units as the band radii and [`outer_gap`](Self::outer_gap), so the
    /// three read against each other directly. One thickness for both
    /// rings: they are one mark seen at two radii, and letting them differ
    /// would say something that isn't true.
    ///
    /// 0 turns the rings off, as a radius of 0 turns the core off. Was
    /// fixed at 0.16 of the band's WIDTH, which moved the rings whenever
    /// the band was resized; absolute holds them still.
    #[serde(default = "default_mark_thickness")]
    pub mark_thickness: f32,

    // ---- Home grid -------------------------------------------------------
    // The faint structural grid between node positions (see `derive_grid`).
    // Its look used to be fixed: the skin's color, a hardcoded inset, a
    // hardcoded thickness, dashes reserved for the sevens links.
    /// Color of the whole idle structure, linear RGBA: the grid lines AND
    /// the idle node markers, which are one visual layer — what carries
    /// the lattice's shape when nothing is playing — and so share a color.
    ///
    /// Alpha is the idle LINE opacity: the strength an unlit home-sheet
    /// segment draws at. The idle markers take only the RGB and keep their
    /// own presence, so dialing the lines faint doesn't quietly dissolve
    /// the markers with them.
    ///
    /// Defaults to the skin's `grid_line`, which is where the lines'
    /// color always came from; the skin has no runtime setter, so a
    /// user-chosen color has to live here. Lit segments still take their
    /// sounding notes' color.
    #[serde(default = "default_grid_color")]
    pub grid_color: [f32; 4],
    /// Grid line thickness as a multiple of the built-in width. 1 is the
    /// classic hairline; the shader scales its grid half-width by this.
    #[serde(default = "default_grid_thickness")]
    pub grid_thickness: f32,
    /// How far a grid segment stops short of each node center, as a factor
    /// of the node radius — the padding between a line end and the
    /// dot/circle drawn there. 0 runs the lines right into the centers;
    /// the old fixed 1.05 sat slightly wider than the disc's visual radius,
    /// so the gap fully contained a sounding note's circle.
    ///
    #[serde(default = "default_grid_inset")]
    pub grid_inset: f32,
    /// Draw the in-plane grid lines dashed. The sevens-axis links are
    /// always dashed regardless — that dash is what distinguishes a depth
    /// link from an in-sheet line, and isn't a style choice.
    #[serde(default)]
    pub grid_dashed: bool,
    // ---- Trail (where the music has already been) ------------------------
    // The one part of the view about the past rather than the present. It
    // rides the IDLE layer only -- see the `trail` module for why that is
    // the whole design and not an implementation detail.
    /// How a node the music has visited is marked (see [`TrailMark`]). Off
    /// by default: showing only what is audible is what every saved view
    /// was drawn with, and leaving marks behind is a deliberate choice.
    #[serde(default)]
    pub trail_mark: TrailMark,
    /// How far the mark departs from a plain idle node, 0..1 — how much
    /// lighter the Lift grey, how visible the Ring, how much color the
    /// Tint. 1 is still quiet by construction; every mark is bounded well
    /// short of reading as a sounding note.
    #[serde(default = "default_trail_strength")]
    pub trail_strength: f32,
    /// Seconds before a pitch is forgotten, measured from when it last
    /// sounded. **0 means never** -- the default, and the point of the
    /// feature: a whole piece's territory rather than a rolling window.
    #[serde(default)]
    pub trail_memory: f32,
    /// Keep the note name and cents on a visited node, not just on sounding
    /// and hovered ones -- so the harmonic space can be read off the screen
    /// by name, with its tuning. Independent of the mark above: the text is
    /// its own channel and is useful with the marks off.
    #[serde(default)]
    pub trail_labels: bool,

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
    /// as a halo around bright notes. 0 disables the chain entirely — the
    /// composite is then exactly the plain scene, so there is deliberately
    /// no separate on/off toggle.
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

// The `default_*` fns below are serde fallbacks for keys a persisted blob
// PREDATES, not the out-of-the-box look — each returns what that blob was
// drawn with before the field existed, so loading an old view doesn't
// restyle it. A fresh view's values live in `impl Default` instead.

/// The classic disc edge radius, from before the core was sizable.
fn default_core_radius() -> f32 {
    0.46
}

/// The classic solid orb — the identity end of the solidity axis.
fn default_core_solidity() -> f32 {
    1.0
}

/// Crisp octave glyphs by default (the classic look, identity end of the
/// outer solidity axis).
fn default_outer_solidity() -> f32 {
    1.0
}

/// The whole circle at full strength: the ring reads as a ring, and the
/// slits alone say which octave owns it.
fn default_mark_unlinked() -> f32 {
    1.0
}

/// What 0.16 of the band's width came to at the default band, which is
/// what the rings were fixed at before this was a bar.
fn default_mark_thickness() -> f32 {
    0.09
}

/// The gap the sectors always had (SLICE_GAP_HALF was 0.06 either side of
/// the boundary), now also the rings' padding from the band.
fn default_outer_gap() -> f32 {
    0.12
}

/// Idle marker at the classic disc radius, so a pre-field blob (whose
/// marker is a Circle) reproduces the old placeholder ring — now
/// independent of the core and of the playing state.
fn default_idle_radius() -> f32 {
    0.46
}

/// The classic annulus (SLICE_IN/OUT, from before the band was
/// parameterized), which is what a blob predating these keys was drawn
/// with. A fresh view uses the wider band in `impl Default` instead.
fn default_outer_inner() -> f32 {
    0.56
}

fn default_outer_outer() -> f32 {
    0.93
}

fn default_render_scale() -> f32 {
    1.0
}

/// Half travel on a bar whose whole range is quiet: clearly a different
/// node, still clearly not a lit one.
fn default_trail_strength() -> f32 {
    0.5
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
    /// (solidity 0) plus the outer layer with its backdrop on. Each of those
    /// bodies once had its own matching glyph shape; only slices survives,
    /// so all three now land there. (Their band radii rode the
    /// slice_inner/slice_outer fields, absorbed by the
    /// outer_inner/outer_outer aliases.)
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

        match std::mem::take(&mut self.node_body) {
            LegacyNodeBody::Disc => return,
            LegacyNodeBody::Slices | LegacyNodeBody::Rings | LegacyNodeBody::Beads => {}
        }
        self.core_solidity = 0.0;
        self.outer_style = OuterStyle::Slices;
        self.outer_backdrop = 1.0;
    }
}

/// The look a fresh view starts in — deliberately NOT the same thing as the
/// `default_*` fns above, which exist to keep a blob saved before a field
/// existed looking the way it did then. Where the two disagree the value is
/// written literally here, so tuning the out-of-the-box look never
/// retroactively restyles someone's saved view.
impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig {
            spacing: 1.0,
            // A tall window of fifths and a wide band of thirds; sevenths
            // start collapsed to the single home sheet (extent 0), so the
            // lattice opens flat and depth is opted into.
            extent_threes: 10,
            extent_fives: 6,
            extent_sevens: 0,
            center_threes: 0,
            center_fives: 0,
            center_sevens: 0,
            outer_style: OuterStyle::Slices,
            show_labels: true,
            show_cents: true,
            node_style: NodeStyle::Checker,
            core_style: CoreStyle::default(),
            // A small, soft core inside a wide octave band with its silent
            // slots ghosted in: the pitch class reads as a compact center
            // and the octaves carry the node's outline.
            core_solidity: 0.4,
            core_radius: 0.2,
            // A narrower octave band, set well off the core and stopping
            // short of the quad edge, with a tight gap between sectors: the
            // octaves read as a ring of distinct marks rather than a solid
            // annulus, and the core keeps clear space around it.
            outer_inner: 0.602_400_54,
            outer_outer: 0.836_332_2,
            outer_backdrop: 0.6,
            legacy_outer_backdrop: None,
            outer_solidity: default_outer_solidity(),
            outer_gap: 0.085_986_16,
            // No idle marker: the grid lines alone carry the lattice's
            // shape where nothing is playing, leaving the node positions
            // themselves empty. (`idle_radius` rides along inert, so
            // switching a marker back on lands at the compact size that
            // matches the core.)
            idle_marker: IdleMarker::None,
            idle_radius: 0.1,
            node_body: LegacyNodeBody::Disc,
            highlight_extremes: HighlightExtremes::default(),
            mark_unlinked: default_mark_unlinked(),
            mark_thickness: 0.082_711_12,
            grid_color: default_grid_color(),
            grid_thickness: default_grid_thickness(),
            grid_inset: 0.3,
            grid_dashed: false,
            // Trail on, with the note names kept on visited nodes. NOTE:
            // Lift works by lightening the idle marker, and the marker is
            // None above, so out of the box the trail shows as the labels
            // alone. See TrailMark::needs_idle_marker.
            trail_mark: TrailMark::Lift,
            trail_strength: default_trail_strength(),
            trail_memory: 0.0,
            trail_labels: true,
            meantone: false,
            frameless: false,
            render_scale: default_render_scale(),
            bloom_strength: 0.5,
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
    /// Seconds a released note keeps fading, for EVERY layer of the node:
    /// the pitch class core, the octave glyphs, and the melody/bass marks.
    /// One time rather than one per layer, so a release reads as a single
    /// gesture instead of layers going dark at different moments.
    pub fade_time: f32,
    /// Pitch (MIDI note) mapped to the darkest gradient color on
    /// pitch-gradient channels.
    pub darkest_pitch: f32,
    /// Pitch mapped to the brightest gradient color.
    pub brightest_pitch: f32,
}

impl Default for FrameParams {
    fn default() -> Self {
        FrameParams {
            fade_time: 1.0,
            darkest_pitch: 24.0,
            brightest_pitch: 108.0,
        }
    }
}
