//! [`ViewConfig`] — the persisted, non-automatable visual settings — its
//! serde defaults and legacy-blob migration, plus the per-frame
//! [`FrameParams`] mirror of the host-automatable appearance parameters.

use crate::skin;
use crate::style::{
    HighlightExtremes, IdleMarker, NodeStyle, PitchGradient, Pulse, SevensLabel,
};
use crate::trail::TrailMark;
use harmonigraph_core::{coords, Comma, LatticePos, Tempered};

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
    /// The curve the low-to-high pitch gradient follows, as its four knobs
    /// (see [`PitchGradient`]). Every pitch-colored shape in the scene reads
    /// it through one table, so this is the only place the gradient is set.
    #[serde(default)]
    pub pitch_gradient: PitchGradient,
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
    /// style fits its glyphs' radial footprint to this. The `slice_inner` /
    /// `slice_outer` spellings these replaced were pre-floor, so no blob
    /// this build accepts carries them and no alias is kept for them;
    /// derive_scene keeps outer ahead of inner, so any dragged
    /// combination still renders a visible band.
    #[serde(default = "default_outer_inner")]
    pub outer_inner: f32,
    #[serde(default = "default_outer_outer")]
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
    /// How many octaves one turn of a node covers at FULL SIZE (see
    /// [`octaves`](crate::octaves)), 1..=11 — not how many it draws, which is
    /// this plus twice [`octave_extras`](Self::octave_extras). Each is exactly
    /// one octave and they all share whatever the extras leave, so this says
    /// how many degrees an octave of the main register is worth. Notes past
    /// either end of the whole wheel light the outermost indicator on their
    /// side.
    #[serde(default = "default_octave_count")]
    pub octave_count: u32,
    /// The MIDI pitch at the TOP of the wheel — on every node, whatever its
    /// pitch class: a node's ring is turned so that its own octaves land on
    /// their pitches, by up to half a slice either way.
    /// [`sanitize`](Self::sanitize) holds it to the settable limits.
    #[serde(default = "default_octave_center")]
    pub octave_center: f32,
    /// The pre-count pitch WINDOW, low end and high, in MIDI note numbers: the
    /// wheel was a continuous pitch range that the octaves were divided out
    /// of, so its ends fell wherever they liked between two of a node's
    /// octaves and the two indicators there were short by however much.
    /// [`sanitize`](Self::sanitize) reads the pair as the count and center it
    /// most nearly named, and never writes it back.
    ///
    /// Both read through `bare_as_some` for the reason the melody/bass shim
    /// above does: the old blobs write the ends BARE, which RON will not read
    /// into an `Option`'s `Some`, and a failed parse drops the whole persist
    /// rather than just this field.
    #[serde(default, skip_serializing, rename = "octave_low", deserialize_with = "bare_as_some")]
    pub legacy_octave_low: Option<f32>,
    /// See [`legacy_octave_low`](Self::legacy_octave_low).
    #[serde(default, skip_serializing, rename = "octave_high", deserialize_with = "bare_as_some")]
    pub legacy_octave_high: Option<f32>,
    /// The pre-window octave COUNT: how many octaves either side of middle C's
    /// the wheel showed, 2..=5, so `2 * span + 1` octaves in all centered on
    /// middle C. Two model generations back, and NOT the count above — a blob
    /// carrying 3 here asked for seven octaves, not three.
    /// [`sanitize`](Self::sanitize) folds it in and never writes it back.
    #[serde(
        default,
        skip_serializing,
        rename = "octave_span",
        deserialize_with = "bare_as_some"
    )]
    pub legacy_octave_span: Option<u32>,
    /// Extra octaves at EACH end of the wheel, drawn small: 0..=5, and never
    /// so many that the whole wheel passes eleven slices. Each one reaches an
    /// octave further up AND down the keyboard for a sliver of the turn, where
    /// an octave of count is paid for by every full-size octave at once.
    ///
    /// A blob written before the fringe existed carries the two bars it
    /// replaced, `octave_taper_amount` and `octave_taper_shape`. Nothing reads
    /// them — an unknown field is ignored rather than refused — so such a
    /// project opens on the same count of octaves it always drew, evenly.
    #[serde(default = "default_octave_extras")]
    pub octave_extras: u32,
    /// How wide one extra is, as a fraction of an EVEN slice (the turn over
    /// the whole wheel, extras included), 0.1..=1. Under 1 an extra is always
    /// narrower than a full-size octave, whatever the count and however many
    /// extras there are, and 1 is an even wheel.
    #[serde(default = "default_octave_extra_size")]
    pub octave_extra_size: f32,
    /// How much the extras GRADE from the outermost inward, 0..1: 0 is a flat
    /// fringe of equal slivers and 1 is a ramp that meets the full-size
    /// octaves in a step the size of its own. The outermost extra is the size
    /// above whatever this is, so it is a shape rather than a second
    /// strength — and it is inert without two extras to differ.
    #[serde(default = "default_octave_extra_blend")]
    pub octave_extra_blend: f32,
    /// Which shimmer sweeps the octave glyphs (see [`Pulse`]) — the pattern
    /// alone, sized and paced by the shared knobs below. It reaches the whole
    /// layer whatever is playing and whatever is marked, so this switch
    /// stands on nothing but itself.
    /// [`Pulse::Off`] is the steady look every earlier build drew, which is
    /// also what a blob predating this field was drawn with — so a bare
    /// `#[serde(default)]` is both fallbacks at once and needs no named
    /// `default_*` fn.
    #[serde(default)]
    pub pulse_octaves: Pulse,
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
    /// Folded into the pair by [`sanitize`](Self::sanitize) and
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
    /// the shape. A [`outer_gap`](Self::outer_gap) of 0 leaves no slit to
    /// draw.
    ///
    /// 0 turns the rings off, as a radius of 0 turns the core off. Was
    /// fixed at 0.16 of the band's WIDTH, which moved the rings whenever
    /// the band was resized; absolute holds them still.
    #[serde(default = "default_mark_thickness")]
    pub mark_thickness: f32,
    /// Which shimmer sweeps the melody/bass rings (see [`Pulse`]): the sheet
    /// takes both rings AND the octave slice each one points at, a quarter
    /// turn from the octave band's own. That reach into the other layer is
    /// the mark being the ring together with the slice it names, so light
    /// crossing one crosses the other. Shares [`Pulse`] with
    /// [`pulse_octaves`](Self::pulse_octaves) but is its own switch — a
    /// chord's outer voices and its octave glyphs are read at a glance
    /// independently, so animating one was never a reason to animate the
    /// other. [`Pulse::Off`] is both the fresh-view and the old-blob
    /// fallback, so a bare `#[serde(default)]` covers it.
    #[serde(default)]
    pub pulse_marks: Pulse,

    // ---- Shimmer ---------------------------------------------------------
    // The sweep's knobs, and ONE set for both layers that can run it. The
    // pattern is per layer because a chord's outer voices and its octave
    // glyphs are read independently; the sizing is not, because it is one
    // sheet of light crossing the whole lattice — two layers sweeping at
    // different sizes or rates would read as two animations stacked on one
    // picture rather than as light passing over it. What the layers do differ
    // in is where the sheet is laid — a quarter turn apart, which is the
    // shader's own constant and not a setting.
    //
    // All four are inert while both layers are Off.
    /// How fast the shimmer travels, in world units per second — the
    /// lattice's own units, so the DAW window and an exported video sweep at
    /// the same rate across the same nodes, where a rate in screen pixels
    /// would not. Which WAY it travels is the pattern's own: along the bands'
    /// normal for the gratings, outward from the origin for
    /// [`Pulse::Rings`]. 0 freezes the sheet where it stands, which is a look
    /// rather than an off switch (the mode is the switch).
    #[serde(default = "default_shimmer_speed")]
    pub shimmer_speed: f32,
    /// How wide the pattern is, in the same world units: the distance from one
    /// bright peak to the next, which sizes the lit part and the dark
    /// between it and its neighbour together — the shimmer is one shape,
    /// scaled, rather than a width and a spacing that could disagree. Every
    /// pattern is built out of gratings of exactly this period, so the bar
    /// means the same thing in all of them (a hex cell comes out about 15%
    /// wider than this, three gratings at sixty degrees being what makes it).
    ///
    /// The range spans three ORDERS of it, and the two ends are different
    /// pictures rather than more and less of one:
    ///
    /// - Wide (around the default, several nodes to a band) is a sheet
    ///   crossing the lattice, each node lighting as it passes.
    /// - Around one node to a band the two read against each other worst:
    ///   neighbours land most of a cycle apart and the picture is alternating
    ///   NODES rather than a band passing over them, the lattice's own
    ///   spacing being irregular (the thirds and fifths axes both project
    ///   onto the screen's x).
    /// - Below that, several bands cross a single node at once and it is a
    ///   texture on the nodes rather than a sweep between them — which is a
    ///   look worth reaching, and why the floor is a small fraction of a node
    ///   rather than a stop above the awkward middle.
    ///
    /// A node is [`spacing`](Self::spacing) × 0.25 in world radius, so the
    /// count of bands across one is roughly its diameter over this.
    ///
    /// The tight end is a resolution trade as well as a look, and the shader
    /// spends it deliberately. A pattern is sines of a world coordinate
    /// sampled once per fragment, so a period approaching a pixel — a tight
    /// setting seen from far enough out — has no samples left to carry it and
    /// would alias into moire that crawls as the camera moves. Rather than
    /// draw that, `shimmer_terms` fades the sheet's amplitude out as its
    /// period closes on the pixel footprint, so the layer settles to its
    /// unshimmered self instead of to a shifting texture. The setting is
    /// still the size it says it is on the lattice; what runs out is the
    /// SAMPLING, and the fade is what makes running out look like an ending
    /// rather than a fault. Frame the shot at the zoom the tight end is
    /// chosen for.
    #[serde(default = "default_shimmer_width")]
    pub shimmer_width: f32,
    /// How strong the sweep is where it passes, 0..1 being none to the full
    /// tuned depth: ONE number drives both of what a band does — how far it
    /// pulls the layer toward white, and how far the layer's coverage dips
    /// between bands — so the two can never be dialed against each other into
    /// a shimmer that brightens without dimming or the reverse.
    ///
    /// 0 is the layer drawing exactly as it does unshimmered, from a bar
    /// rather than from the mode. Past 1 the two stop moving together, and
    /// not because they are separate: the white mix RUNS OUT at about 1.18,
    /// where a band peak reaches white and there is nothing whiter to reach
    /// for, while the trough goes on deepening to the clamp. So the top of
    /// the bar buys its contrast by darkening the layer between bands rather
    /// than by lighting the band — still more shimmer, and worth having, but
    /// a different trade from the bottom half.
    ///
    /// What the light costs is real at any setting, and it is the point of
    /// the bar: under a strong band an indicator says "an octave sounds here"
    /// without saying which.
    #[serde(default = "default_shimmer_intensity")]
    pub shimmer_intensity: f32,
    /// How the light is shared out ACROSS one period, 0..1 — where
    /// [`shimmer_intensity`](Self::shimmer_intensity) says how much light
    /// there is, this says how gradually it arrives.
    ///
    /// The pattern is a raised cosine raised to a power, and this is the
    /// power, log-spaced from 8 at 0 to 1 at 1:
    ///
    /// - Toward 0 the peak is a narrow crest on a layer that is otherwise at
    ///   rest — a hard white band with a dark field around it, which at a
    ///   tight width is a stripe pattern more than a sweep.
    /// - Toward 1 the exponent reaches 1 and the pattern IS the cosine: every
    ///   point of the period is on its way somewhere, so the brightest part
    ///   fades into the clearest across the whole of the gap rather than at
    ///   an edge. Nothing is at rest, which is the cost — the layer is lit
    ///   somewhere at every instant.
    ///
    /// One number for both halves of the shape, like Intensity: the bright
    /// part narrows exactly as the dark part widens, so a period always adds
    /// up to itself and no setting can leave the sheet mostly-lit and
    /// mostly-dark at once.
    #[serde(default = "default_shimmer_softness")]
    pub shimmer_softness: f32,

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
    /// (temper out the syntonic comma, 81/80). While on, the third-tuning
    /// value is derived from the fifth (in `begin_frame`) and note names are
    /// respelled without their comma marks.
    ///
    /// One of two comma switches, and the pattern for both: the flag is named
    /// after the temperament that tempers its comma out, [`Self::marvel`] is
    /// the same switch for 225/224, and [`ViewConfig::tempers`] is how the UI
    /// reaches either by [`Comma`] rather than by name.
    ///
    /// Whether this engages by itself is [`Self::meantone_auto`]'s business;
    /// releasing it is always an edit of the major third (or this switch,
    /// while the auto-detect is off).
    #[serde(default)]
    pub meantone: bool,
    /// Auto-detect meantone: engage [`Self::meantone`] whenever the tuning
    /// params land within `TEMPER_TOLERANCE` of the meantone identity —
    /// however they got there (a learned chord, the 12-TET preset, a drag
    /// of either bar). The major third then snaps to four perfect fifths
    /// and the comma marks go.
    ///
    /// Engage-only, deliberately: the lock has to survive dragging the
    /// FIFTH, which moves the derived third out from under a third param
    /// that is inert while the lock holds. So the release is the one edit
    /// that can mean nothing else — pulling the major third itself more
    /// than the tolerance away from the derived value.
    ///
    /// On by default, and `default_true` so a blob written before it
    /// existed opts in too: a project saved at 12-TET (400 = 4·700 − 2400)
    /// is meantone whether or not anyone said so, and its E and E- name one
    /// pitch. Switching this off leaves the mode wherever it is and hands
    /// the switch back.
    #[serde(default = "default_true")]
    pub meantone_auto: bool,
    /// Marvel mode: lock the harmonic-seventh tuning to two fifths plus two
    /// thirds (temper out the septimal kleisma, 225/224). The same switch as
    /// [`Self::meantone`] one prime up — while on, the seventh-tuning value
    /// is derived in `begin_frame` and the sevens sheet is respelled onto the
    /// home sheet, where a harmonic seventh reads `A♯-2` (two fifths plus two
    /// thirds) instead of `B♭↓`.
    ///
    /// The third it derives from is the one in USE, so with meantone on too
    /// the pair composes into septimal meantone (a seventh of ten fifths) and
    /// every name on the lattice comes out a plain letter.
    #[serde(default)]
    pub marvel: bool,
    /// Auto-detect marvel: [`Self::meantone_auto`]'s twin, engage-only for
    /// the same reason — the lock has to survive dragging the fifth or the
    /// third, either of which moves the derived seventh out from under a
    /// seventh param that is inert while the lock holds.
    ///
    /// On by default, and `default_true` on the same grounds as the meantone
    /// detect: 12-TET tempers 225/224 out as well (1000 = 2·700 + 2·400 −
    /// 1200), so a project saved there has one pitch under `B♭↓` and `A♯`
    /// whether or not anyone said "marvel". A blob written before this
    /// existed therefore opts in, and reopening it respells the sevens sheet
    /// — which is the tuning's own arithmetic finally showing up in the
    /// names, not a change of mind about the project.
    #[serde(default = "default_true")]
    pub marvel_auto: bool,
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
    /// Off by default: the HUD is a development instrument, and it sits over
    /// the picture the plugin exists to draw. The Panel pane's Performance
    /// section is where it gets switched on.
    ///
    /// Plain `default`, matching the struct default, so a fresh install and a
    /// pre-`show_perf` blob both open with it off. A project that explicitly
    /// turned it on carries `true` and still round-trips.
    #[serde(default)]
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

/// The rate the sweep was fixed at before it was a bar, and where a fresh
/// view opens too: about one band every three seconds at the default width,
/// which is the calm end of what still reads as moving.
fn default_shimmer_speed() -> f32 {
    1.6
}

/// The band size the sweep was fixed at, and the fresh-view value: about five
/// nodes at the default spacing, so a band spans several of them and reads as
/// one crossing the lattice rather than as a marking on each.
fn default_shimmer_width() -> f32 {
    5.0
}

/// The full tuned depth — what the sweep was fixed at before the bar, and
/// where a fresh view opens: a band most of the way to white over a shallow
/// dip, which is the balance the two shader constants were dialed to.
fn default_shimmer_intensity() -> f32 {
    1.0
}

/// Well up the gradual half of the bar (an exponent of about 1.5): the peak
/// reads as one place the light is brightest rather than as a band with
/// edges, and the fall from it takes most of the period.
///
/// The alternative is a crest — anywhere below about 0.6, where the exponent
/// passes 2.4 and the lit part narrows to a fraction of the period. That
/// reads as white stripes laid ON the layer rather than as light crossing it,
/// the more so the tighter the width, and it is the whole reason this is a
/// setting rather than a constant.
fn default_shimmer_softness() -> f32 {
    0.8
}

/// Nine octaves, which is what a blob written before the wheel was a setting
/// at all was drawn with: ten fixed 45-degree sectors covering MIDI octaves
/// 0..9. Nine of them is the nearest honest reading of that, and unlike the
/// ten it divides the circle evenly instead of wrapping the top two octaves
/// back over the bottom two. A blob written against the pitch WINDOW carries
/// `octave_low`/`octave_high` instead, and one written against the octave
/// COUNT carries `octave_span`; `sanitize` folds both in.
fn default_octave_count() -> u32 {
    9
}

/// Middle C, which every wheel this predates was centered on.
fn default_octave_center() -> f32 {
    crate::octaves::DEFAULT_CENTER
}

/// No fringe: the count is the whole of what a blob written before the extras
/// asked for, so it opens on an even wheel and any fringe is a choice made
/// from there.
fn default_octave_extras() -> u32 {
    0
}

/// The size an extra took before the size was settable, and inert until
/// there is one. A fresh view opens on a wider fringe than this.
fn default_octave_extra_size() -> f32 {
    crate::octaves::DEFAULT_EXTRA_SIZE
}

/// A flat fringe — the bottom of the Blend bar, and inert until there are two
/// extras to grade between.
fn default_octave_extra_blend() -> f32 {
    crate::octaves::DEFAULT_EXTRA_BLEND
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
    /// Whether a melody/bass ring can be drawn at all: an end has to be
    /// marked for there to BE a ring, and the thickness has to leave it
    /// something to draw with.
    ///
    /// One predicate because three places have to agree on it — the pane
    /// grays the ring's own controls, `derive_scene` folds
    /// [`pulse_marks`](Self::pulse_marks) off, and the Shimmer bars gray with
    /// whatever is left running — and three copies of a two-term condition is
    /// how they come to disagree. They did: the fold tested the thickness
    /// alone, so a saved view with both marks off kept shipping a live pulse
    /// mode, and the Shimmer bars tested neither and stayed draggable with
    /// nothing to drag.
    ///
    /// Says nothing about whether a ring is drawn NOW — that is a held note's
    /// business, per node. This is whether the layer is switched on.
    pub fn mark_rings_draw(&self) -> bool {
        self.mark_thickness > 0.0 && (self.mark_melody || self.mark_bass)
    }

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

    /// The commas being tempered out, as the set a name is spelled against
    /// ([`LatticePos::respell`]). The flags are stored one per comma so a
    /// saved project keeps reading, and this is where they become the one
    /// value every naming path takes.
    pub fn tempered(&self) -> Tempered {
        Tempered { syntonic: self.meantone, septimal_kleisma: self.marvel }
    }

    /// Whether one comma is being tempered out.
    pub fn tempers(&self, comma: Comma) -> bool {
        match comma {
            Comma::Syntonic => self.meantone,
            Comma::SeptimalKleisma => self.marvel,
        }
    }

    /// Whether one comma's auto-detect is running.
    pub fn temper_auto(&self, comma: Comma) -> bool {
        match comma {
            Comma::Syntonic => self.meantone_auto,
            Comma::SeptimalKleisma => self.marvel_auto,
        }
    }

    /// The switch for one comma's tempering, to read or set. Together with
    /// [`Self::temper_auto_mut`] this is what lets the tempering section be a
    /// loop over [`Comma::ALL`] instead of a block per comma.
    ///
    /// A third comma is then additive rather than another special case, but
    /// it is not free: the variant and its arms on [`Comma`], two fields and
    /// four arms here, one in `LatticePos::respell`, one in the UI's
    /// `judged_axes`, and one in its `derived_key` — which lives there
    /// because a `ParamKey` is the UI's to name, not core's.
    pub fn temper_mut(&mut self, comma: Comma) -> &mut bool {
        match comma {
            Comma::Syntonic => &mut self.meantone,
            Comma::SeptimalKleisma => &mut self.marvel,
        }
    }

    /// The auto-detect switch for one comma.
    pub fn temper_auto_mut(&mut self, comma: Comma) -> &mut bool {
        match comma {
            Comma::Syntonic => &mut self.meantone_auto,
            Comma::SeptimalKleisma => &mut self.marvel_auto,
        }
    }

    /// Fit a deserialized view to what its controls can actually produce, and
    /// fold the fields a loadable blob may still spell an older way.
    ///
    /// The clamping is not about old blobs: a bar cannot produce a nonsense
    /// value but a hand-edited RON can, and these feed a rasterizer.
    pub fn sanitize(&mut self) {
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

        // The pitch gradient's four knobs, for the same reason as the label
        // scale above and one more: they are the memo key of the color table
        // every pitch-colored shape reads, so a non-finite one would miss the
        // cache on every lookup as well as drawing a NaN.
        self.pitch_gradient = self.pitch_gradient.sanitized();

        // The melody/bass marks' pre-split enum, which was exactly these two
        // bits packed into four names.
        if let Some(which) = self.legacy_highlight_extremes.take() {
            self.mark_melody = which.marks_melody();
            self.mark_bass = which.marks_bass();
        }

        // The octave wheel's two earlier models, oldest first so the newer one
        // wins if a blob somehow carries both.
        //
        // The pre-window COUNT named `2 * span + 1` octaves centered on middle
        // C, which the count and center say directly.
        if let Some(span) = self.legacy_octave_span.take() {
            // 2..=5 because that is what the layout clamped the count to, and
            // a blob outside it was drawn at the clamp.
            self.octave_count = 2 * span.clamp(2, 5) + 1;
            self.octave_center = 60.0;
        }
        // The pre-count pitch WINDOW, read as the wheel that most nearly draws
        // it: the window's own middle was the pitch at the top, so it becomes
        // the center, and the octaves it spanned become the count. The window
        // was continuous, so that count is generally a rounding — a project
        // whose window was nine and a half octaves wide has to open on one of
        // the two whole numbers either side of it, and the half octave it
        // loses is exactly the sliver that used to cut its end indicators
        // short.
        let window = (self.legacy_octave_low.take(), self.legacy_octave_high.take());
        if let (Some(low), Some(high)) = window {
            if low.is_finite() && high.is_finite() {
                let (low, high) = if low <= high { (low, high) } else { (high, low) };
                self.octave_center = 0.5 * (low + high);
                self.octave_count = ((high - low) / 12.0).round().max(0.0) as u32;
            }
        }
        // Together, because the pair is what has to fit the boundary table and
        // either one alone can be legal in a wheel that isn't.
        (self.octave_count, self.octave_extras) =
            crate::octaves::clamp_wheel(self.octave_count, self.octave_extras);
        self.octave_center = crate::octaves::clamp_center(self.octave_center);

        // The fringe feeds the wheel's boundary angles, and a non-finite size
        // or blend poisons every one of them: the widths come out NaN, so does
        // each `cos`/`sin` in the shader, and the whole octave layer vanishes
        // with nothing to say why. `clamp` alone does not catch it — NaN is
        // its own answer — hence the finite check either side of it.
        self.octave_extra_size = if self.octave_extra_size.is_finite() {
            self.octave_extra_size.clamp(crate::octaves::MIN_EXTRA_SIZE, 1.0)
        } else {
            default_octave_extra_size()
        };
        self.octave_extra_blend = if self.octave_extra_blend.is_finite() {
            self.octave_extra_blend.clamp(0.0, 1.0)
        } else {
            default_octave_extra_blend()
        };

        // The shimmer's four knobs, on the same grounds and against the same
        // hole in `clamp`. `derive_scene` clamps all four into their ranges
        // every frame, which is what the shader trusts — and a NaN walks
        // through a clamp untouched, because every comparison against it is
        // false. From there it is a divide (the period), a `pow` exponent
        // (the softness) and two mixes, so ONE non-finite number in a
        // hand-edited blob NaNs the sheet, and a NaN sheet takes the whole
        // octave layer with it wherever the mode is on. Repaired here rather
        // than in `derive_scene` because this is the blob's own door: the bars
        // cannot reach these values, so a view that holds one got it from a
        // file.
        self.shimmer_speed = finite_or(self.shimmer_speed, default_shimmer_speed());
        self.shimmer_width = finite_or(self.shimmer_width, default_shimmer_width());
        self.shimmer_intensity = finite_or(self.shimmer_intensity, default_shimmer_intensity());
        self.shimmer_softness = finite_or(self.shimmer_softness, default_shimmer_softness());
    }
}

/// `value` if it is a real number, and `fallback` if it is a NaN or an
/// infinity — the guard `clamp` cannot be, NaN being its own answer to every
/// comparison a clamp makes.
///
/// No range: the caller's own clamp is the range, and this only has to hand
/// it something a clamp can act on.
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
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
            // sevens sheet alone. A sheet either side (extent 1) shows the
            // septimal axis without anyone having to go find it; the
            // tradeoff is that nothing tells the eye which sheet a node is
            // on until the sevens layer settings below are turned down to
            // read as an annotation rather than a second sheet (see
            // sevens_size).
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
            pitch_gradient: PitchGradient::default(),
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
            outer_inner: 0.661_417_3,
            outer_outer: 0.851_483_05,
            outer_gap: 0.051_732_67,
            // Five octaves to the turn with middle C straight up — C1..C5 in
            // the DAW's numbering, the register a keyboard part lives in, at
            // 72 degrees an octave, with a two-octave fringe either end (see
            // octave_extras) narrower than a full-size slice and graded from
            // the outer edge in.
            octave_count: crate::octaves::DEFAULT_COUNT,
            octave_center: crate::octaves::DEFAULT_CENTER,
            legacy_octave_low: None,
            legacy_octave_high: None,
            legacy_octave_span: None,
            octave_extras: 2,
            octave_extra_size: 0.387_534_47,
            octave_extra_blend: 0.562_241_4,
            // Steady by default: the pulse is an option to reach for, not
            // the out-of-the-box look.
            pulse_octaves: Pulse::Off,
            // No idle marker: the grid lines alone carry the lattice's
            // shape where nothing is playing, leaving the node positions
            // themselves empty. (`idle_radius` rides along inert, so
            // switching a marker back on lands at the compact size that
            // matches the core.)
            idle_marker: IdleMarker::None,
            idle_radius: 0.1,
            // Both ends marked: the rings are subtle enough to live with
            // always on, and a chord's outer voices are worth seeing without
            // having to go turn something on first.
            mark_melody: true,
            mark_bass: true,
            legacy_highlight_extremes: None,
            // Thin rings, slit at the marked octave's boundaries.
            mark_thickness: 0.063_829_795,
            // Steady here too, for the same reason as pulse_octaves above:
            // an option to reach for, not the out-of-the-box look.
            pulse_marks: Pulse::Off,
            // The sweep opens on the size and pace the mode was tuned at, so
            // switching a layer to a pattern lands on a look rather than on a
            // setting to find first; the bars are then a departure from it.
            shimmer_speed: default_shimmer_speed(),
            shimmer_width: default_shimmer_width(),
            shimmer_intensity: default_shimmer_intensity(),
            shimmer_softness: default_shimmer_softness(),
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
            meantone_auto: true,
            marvel: false,
            marvel_auto: true,
            frameless: false,
            show_perf: false,
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
