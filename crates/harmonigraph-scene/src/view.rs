//! [`ViewConfig`] — the persisted, non-automatable visual settings — its
//! serde defaults and legacy-blob migration, plus the per-frame
//! [`FrameParams`] mirror of the host-automatable appearance parameters.

use crate::skin;
use crate::style::{
    CoreStyle, HighlightExtremes, IdleMarker, LegacyNodeBody, NodeStyle, SevensLabel,
};
use crate::trail::TrailMark;
use harmonigraph_core::{coords, LatticePos};

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
    // ---- The sevens layer ------------------------------------------------
    // How the sheets other than the home one draw. `sevens_size` and
    // `sevens_label` go inert while `extent_sevens` is 0, which is where a
    // fresh view starts; `sevens_gutter` does NOT — it is named for this
    // layer but cuts the grid under a sounding node at any extent (see its
    // own doc below, and `a_flat_lattice_still_clears_its_grid`).
    //
    // The problem all three settings answer: the 5-limit sheet wants its
    // pitch classes as large as they will go, and at the default spacing a
    // node's visible edge already reaches 0.376 of the way to its neighbor.
    // Turning depth on asks the same rectangle to hold three times the
    // nodes. Something has to give, and it must not be the home sheet — that
    // is the picture.
    /// How much smaller a node draws for each step it sits off the home
    /// sheet: the factor is `sevens_size^|sevens - center_sevens|`. 1 keeps
    /// every sheet the same size.
    ///
    /// Smaller in BOTH directions, deliberately, even though a positive
    /// sevens step is the one nearer the camera — this is not perspective.
    /// Size here says *how far from the home sheet*, because that is the
    /// thing worth reading: the home sheet is the ground the music is heard
    /// against, so it stays the largest thing on screen whichever way the
    /// sevens axis runs.
    #[serde(default = "default_sevens_size")]
    pub sevens_size: f32,
    /// Width of the dark gutter a node clears around itself, in quad UV
    /// units — the units a FULL-SIZE node uses, so the gap comes out the
    /// same width on screen whatever size the node it belongs to draws at.
    /// (The shader divides by the node's size factor to get there.) A gap
    /// that shrank with its node read as a property of the note rather than
    /// of the layer it sits on. 0 draws none.
    ///
    /// This is what lets the sevens layer OVERLAP the home sheet instead of
    /// needing room of its own: the node punches its own footprint out of
    /// whatever it crosses and sits in the hole, so a small node stays
    /// legible over a large one. It costs no layout space at all, which is
    /// the whole point — the alternative is shrinking the 5-limit sheet to
    /// open up clearance, and the 5-limit sheet is what you came to look at.
    ///
    /// It clears to the GROUND the pass is composited over, which the shell
    /// hands in (see [`Scene::background`](crate::Scene::background)). With
    /// no color of its own a premultiplied layer knocks out to black, and
    /// black is several shades darker than this skin's panel, so the
    /// clearing announced itself as a plate sitting on the picture rather
    /// than disappearing into the ground.
    ///
    /// It fades rather than ending at a rim, over a band twice this wide —
    /// a hard circle cutting across a lit ring reads as a bite taken out of
    /// it. And its STRENGTH is the note's own envelope (applied in the
    /// shader, against the same activation that paints the node), so a
    /// clearing fades out exactly as its note does while holding its width;
    /// a node that sounds nothing clears nothing at all.
    ///
    /// Named for the sevens layer it was built for, but not confined to it:
    /// the home sheet clears too, and at any sevenths extent — with the
    /// lattice flat there is no sheet behind to hide, but the grid lines
    /// are still cut, which is a look worth having on its own.
    #[serde(default)]
    pub sevens_gutter: f32,
    /// How wide the clearing's fade is, same units — and deliberately NOT
    /// derived from the reach above. Tying the two into one number (fade
    /// pinned to twice the reach) means widening the gap also blurs it,
    /// with no way to ask for a wide crisp one or a narrow soft one.
    ///
    /// The clearing is solid out to `reach - fade` past the node's rim and
    /// gone by `reach`, so the reach is exactly where it ends whatever the
    /// fade does. Floored at the node's own rim: a fade wider than the
    /// reach would otherwise start eating into the node's own footprint,
    /// which is the one part that must always be cleared.
    #[serde(default = "default_sevens_gutter_soft")]
    pub sevens_gutter_soft: f32,
    /// What text an off-sheet node's label carries (see [`SevensLabel`]).
    /// Only meaningful while `show_labels` is on.
    #[serde(default = "default_sevens_label")]
    pub sevens_label: SevensLabel,
    /// Stroke weight of the DRAWN label marks (`+`, `-`, and the septimal
    /// shape), as a fraction of the mark's own font size.
    ///
    /// Draw note-name labels on hovered and sounding nodes.
    /// serde(default) keeps older persisted blobs loadable.
    #[serde(default = "default_true")]
    pub show_labels: bool,
    /// Overall size of a node's label, as a multiple of its built-in sizes —
    /// the note name, the marks stacked beside it and the cents line under it
    /// together, so the label keeps its proportions and only the whole of it
    /// grows.
    ///
    /// It trims what the CAMERA decides rather than replacing it. A label
    /// tracks the on-screen size of the lattice it sits on (see
    /// [`Camera::screen_scale`](crate::Camera::screen_scale)), which is what
    /// keeps a name the same size on its node at every zoom; this says what
    /// that size is.
    #[serde(default = "default_label_scale")]
    pub label_scale: f32,
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
    // The octave layer's backdrop and solidity are fixed at 1 in the shader
    // and have no fields here. The backdrop — the silent octaves ghosted
    // faintly behind the sounding sectors, in the note color — is what makes
    // the annulus complete, so a lone octave still reads as a whole note;
    // and the glyphs are always the crisp classic shapes. Saved blobs may
    // still carry the keys these rode on (`outer_backdrop`, first a bool and
    // then an opacity under `outer_backdrop_alpha`, and `outer_solidity`);
    // serde ignores unknown keys, so such a blob loads intact and simply
    // drops them on the next save.
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
    // Mark the outer held notes, so the melody and/or bass line reads at a
    // glance out of a chord. "Outer" is by sounding pitch (`Voice::pitch`,
    // which includes MPE/tuning bends), over HELD voices only: a released
    // note is on its way out and shouldn't keep the mark from the note that
    // replaced it.
    //
    // A mark rides the OUTER EDGE of that note's octave indicator and
    // nothing else — it never touches the core, whose color is the note's
    // own. That also makes it the layer that survives a chord voiced within
    // a single pitch class: every octave of one note lands on the same node,
    // differing only by slot.
    /// Mark the highest held note.
    ///
    /// Independent of [`mark_bass`](Self::mark_bass), which is what the two
    /// of them are: the rings are told apart by radius (melody inside the
    /// octave band, bass outside) rather than by hue, so a note that is at
    /// once the highest and the lowest — a lone held note, or a chord whose
    /// top and bottom share a pitch class — simply gets both.
    #[serde(default = "default_true")]
    pub mark_melody: bool,
    /// Mark the lowest held note. See [`mark_melody`](Self::mark_melody).
    #[serde(default = "default_true")]
    pub mark_bass: bool,
    /// Load-only shim: blobs from before the two marks became independent
    /// flags carry one `highlight_extremes` token (Off/Melody/Bass/Both).
    /// Folded into the pair by [`migrate_legacy`](Self::migrate_legacy) and
    /// never written back. It reads through `bare_as_some` because the old
    /// blobs wrote the variant BARE, which RON will not read into an
    /// `Option`'s `Some`, and a failed parse drops the whole persist rather
    /// than just this field — losing the user's layout and camera too.
    #[serde(
        default,
        skip_serializing,
        rename = "highlight_extremes",
        deserialize_with = "bare_as_some"
    )]
    pub legacy_highlight_extremes: Option<HighlightExtremes>,
    /// How thick each melody/bass ring is, in quad UV units — the same
    /// units as the band radii and [`outer_gap`](Self::outer_gap), so the
    /// three read against each other directly. One thickness for both
    /// rings: they are one mark seen at two radii, and letting them differ
    /// would say something that isn't true.
    ///
    /// A ring is a whole circle, slit at the two sector boundaries of the
    /// octave responsible for it — the slit IS the gap between two octaves,
    /// continued outward, so the ring says which octave without giving up
    /// the shape. (An `Unlinked` opacity used to fade everything but that
    /// arc; it is fixed at full now, which is the ring reading as a ring.)
    /// A [`outer_gap`](Self::outer_gap) of 0 leaves no slit to draw.
    ///
    /// 0 turns the rings off, as a radius of 0 turns the core off. Was
    /// fixed at 0.16 of the band's WIDTH, which moved the rings whenever
    /// the band was resized; absolute holds them still.
    #[serde(default = "default_mark_thickness")]
    pub mark_thickness: f32,

    // ---- Home grid -------------------------------------------------------
    // The faint structural grid between node positions (see `derive_grid`).
    // Color, inset, thickness and dashing are all settings; the fields
    // below are the whole of its look.
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
    /// 1.05 sits slightly wider than the disc's visual radius, so the gap
    /// fully contains a sounding note's circle.
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
    /// Hide every tab bar so adjacent panes — lattice above spectrum, in the
    /// default layout — record as one seamless surface. Tab toggles it.
    ///
    /// The separators keep their regular width, so the spacing between panes
    /// is the same in both modes and a take framed in one is framed in the
    /// other.
    #[serde(default)]
    pub frameless: bool,
    /// Show the performance overlay (a small corner HUD with frame rate,
    /// memory and workload counts; per-stage CPU time waits for
    /// [`Self::show_perf_detail`]). Interactive shells only — the offline
    /// renderer never draws it, keeping its frames deterministic.
    ///
    /// `default_true`, matching the struct default, so a fresh install and a
    /// pre-`show_perf` blob both open with it on. A project that explicitly
    /// turned it off carries `false` and still round-trips.
    #[serde(default = "default_true")]
    pub show_perf: bool,
    /// Expand the overlay from the headline numbers into the full per-stage
    /// breakdown of where a frame goes.
    ///
    /// Off by default: the breakdown exists to answer "which stage is eating
    /// the frame", and once it has, a dozen rows of scaffolding is not what
    /// you want sitting over the picture. Inert while `show_perf` is off.
    #[serde(default)]
    pub show_perf_detail: bool,
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

/// Read a legacy BARE value — `true`, `Both` — into an `Option<T>` that
/// means "the key was there". A plain `Option<T>` field can't do this: RON
/// writes options as `Some(true)`/`None`, so it rejects the bare token the
/// old blobs actually contain. `serde(default)` still supplies `None` when
/// the key is absent — this only runs when it is present.
fn bare_as_some<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
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

/// Every sheet the same size — what a blob predating the sevens layer's own
/// settings was drawn with. (`sevens_gutter` takes the plain `0` default for
/// the same reason: no gutter existed to draw.)
fn default_sevens_size() -> f32 {
    1.0
}

/// The fade a blob predating this key was drawn with, near enough: it was
/// pinned to twice the reach, and the reach it shipped with was 0.12.
fn default_sevens_gutter_soft() -> f32 {
    0.24
}

/// The note name, which every earlier build drew off the home sheet too —
/// but unambiguously now, the septimal mark having given an off-sheet name
/// its own spelling rather than its namesake's.
fn default_sevens_label() -> SevensLabel {
    SevensLabel::Name
}

/// Labels at their built-in size.
///
/// The deliberate exception to what these `default_*` fns are for. An older
/// blob was drawn at HALF this — the letter was 15pt and is now 30 — so this
/// restyles it, where every other default here exists precisely not to. The
/// bar this field belongs to went to 2 the first time it was tried and stayed
/// there, which said the built-in size was wrong rather than merely one
/// choice; a size that was wrong is worth correcting everywhere it was used,
/// and a saved view that kept it would be preserving a mistake, not a look.
fn default_label_scale() -> f32 {
    1.0
}

/// The classic solid orb — the identity end of the solidity axis.
fn default_core_solidity() -> f32 {
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
/// with. A fresh view uses a narrower band, set further off the core, in
/// `impl Default` instead.
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
    /// (solidity 0), the outer layer carrying the note alone. Each of those
    /// bodies once had its own matching glyph shape; only slices survives,
    /// so all three now land there. (Their band radii rode the
    /// slice_inner/slice_outer fields, absorbed by the
    /// outer_inner/outer_outer aliases.)
    pub fn migrate_legacy(&mut self) {
        // Fit the label scale to what its bar offers. It multiplies a FONT
        // SIZE, and the bar cannot produce a nonsense value where a
        // hand-edited blob can: a non-finite one reaches egui as a glyph with
        // no image, so every label silently vanishes, and a huge one asks the
        // rasterizer for a glyph wider than the texture atlas can hold.
        self.label_scale = if self.label_scale.is_finite() {
            self.label_scale.clamp(0.3, 3.0)
        } else {
            default_label_scale()
        };

        // The melody/bass marks' pre-split enum, which was exactly these two
        // bits packed into four names.
        if let Some(which) = self.legacy_highlight_extremes.take() {
            self.mark_melody = which.marks_melody();
            self.mark_bass = which.marks_bass();
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
            // A tall window of fifths and a wide band of thirds, on the home
            // sevens sheet alone. Opening with a sheet either side (extent 1)
            // shows the septimal axis without anyone having to go find it,
            // but it also puts a second sheet in the picture before anything
            // tells the eye which sheet a node is on; flat is the look this
            // is actually used at. The sevens layer settings below are the
            // lever for making depth readable, but they need setting by hand
            // when it is opened rather than being sized for it already.
            extent_threes: 10,
            extent_fives: 6,
            extent_sevens: 0,
            center_threes: 0,
            center_fives: 0,
            center_sevens: 0,
            // Sevens sheets at full size, which rides along inert while the
            // axis is collapsed above. At full size a sheet rivals the home
            // one rather than annotating it — nothing says which sheet a node
            // is on and an off-sheet label lands on its neighbours — so
            // opening depth is also the cue to bring this down (around 0.55
            // reads as an annotation). Its label keeps the name, which the
            // septimal mark spells apart from the node two fifths down (see
            // SevensLabel) rather than repeating it.
            sevens_size: 1.0,
            // Reach and fade equal, which puts the clearing at full strength
            // exactly to the node's rim and gone a quarter of a node-width
            // past it.
            sevens_gutter: 0.24,
            sevens_gutter_soft: 0.24,
            sevens_label: SevensLabel::Name,
            show_labels: true,
            label_scale: default_label_scale(),
            show_cents: true,
            node_style: NodeStyle::Steady,
            core_style: CoreStyle::default(),
            // A small, soft core inside the octave band, with the band's
            // silent slots ghosted in: the pitch class reads as a compact
            // center and the octaves carry the node's outline. (The band's
            // own width is set below.)
            core_solidity: 0.4,
            core_radius: 0.2,
            // A narrow octave band, set well off the core and stopping short
            // of the quad edge, with a tight gap between sectors: the octaves
            // read as a ring of distinct marks rather than a solid annulus,
            // and the core keeps clear space around it. (The backdrop that
            // holds the whole ring's shape behind them is fixed on.)
            outer_inner: 0.641_313_55,
            outer_outer: 0.851_483_05,
            outer_gap: 0.051_732_67,
            // No idle marker: the grid lines alone carry the lattice's
            // shape where nothing is playing, leaving the node positions
            // themselves empty. (`idle_radius` rides along inert, so
            // switching a marker back on lands at the compact size that
            // matches the core.)
            idle_marker: IdleMarker::None,
            idle_radius: 0.1,
            node_body: LegacyNodeBody::Disc,
            // Both ends marked: the rings are subtle enough to live with
            // always on, and a chord's outer voices are worth seeing without
            // having to go turn something on first.
            mark_melody: true,
            mark_bass: true,
            legacy_highlight_extremes: None,
            // Thin rings, slit at the marked octave's boundaries.
            mark_thickness: 0.063_829_795,
            grid_color: default_grid_color(),
            grid_thickness: 1.103_806_3,
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
            show_perf: true,
            show_perf_detail: false,
            render_scale: default_render_scale(),
            // A halo just under unit strength: the small soft core and the
            // thin octave marks are quiet shapes, and the bloom is what
            // gives them presence.
            bloom_strength: 0.974_009_9,
        }
    }
}

/// Per-frame mirrors of the host-automatable appearance parameters. The
/// shell copies these from its param backend every frame (see root_ui).
/// Deliberately NOT part of [`ViewConfig`] or the persist blob: the param
/// system owns these values, and persisting a copy would create a second
/// source of truth that's dead on arrival at load time.
#[derive(Clone, Copy, Debug, PartialEq)]
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
