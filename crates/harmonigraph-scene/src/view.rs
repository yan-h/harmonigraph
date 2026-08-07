//! [`ViewConfig`] — the persisted, non-automatable visual settings — plus the
//! per-frame [`FrameParams`] mirror of the host-automatable appearance
//! parameters.

use crate::skin;
use crate::style::{IdleMarker, PitchGradient, Pulse, SevensLabel};
use crate::trail::TrailMark;
use harmonigraph_core::{coords, Comma, Envelope, LatticePos, Tempered};

/// Purely-visual settings (not host-automatable parameters). The UI layer
/// persists these separately from plugin parameters.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
// Every field falls back to `impl Default` below, so a blob missing one costs
// that key alone rather than failing to parse and taking the whole persist —
// the camera, the dock and every other setting — down with it.
#[serde(default)]
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
    pub center_threes: i32,
    pub center_fives: i32,
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
    pub sevens_gutter_soft: f32,
    /// What text an off-sheet node's label carries (see [`SevensLabel`]).
    /// Only meaningful while `show_labels` is on.
    pub sevens_label: SevensLabel,
    /// Draw note-name labels on hovered and sounding nodes.
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
    pub label_scale: f32,
    /// Under each note-name label, also show the node's pitch class in
    /// cents. Only meaningful while `show_labels` is on.
    pub show_cents: bool,
    // How a sounding node's core is painted has no field here: the core is
    // the steady disc-and-glow, and nothing switches it. The field styles
    // that stood beside it (Vortex, Checker and Spiral) are gone, and with
    // them the `node_style` key and the per-node seed that animated them.
    // Saved blobs still carry that key, naming any of the SEVENTEEN the enum
    // answered to — those three, the Steady it defaulted to, and the thirteen
    // trimmed before them that its serde aliases went on loading (Breathe,
    // Sparks, Wire, Corona, Plasma, Aurora, Marble, Lava, Filament, Stripes,
    // Rings, Tiles, Pinwheel). Serde ignores unknown keys, so such a blob
    // loads intact and drops the key on the next save. This is the only
    // surviving record of that set, which is why it names it in full.
    /// The curve the low-to-high pitch gradient follows, as its four knobs
    /// (see [`PitchGradient`]). Every pitch-colored shape in the scene reads
    /// it through one table, so this is the only place the gradient is set.
    pub pitch_gradient: PitchGradient,
    /// The core's solidity, 0..1: a soft glow at 0, morphing continuously
    /// to the classic solid orb at 1 (the disc fades in over its glow
    /// skirt and its edge crisps). Inert while the core is off.
    pub core_solidity: f32,
    /// The core's radius, in quad UV units (0 = node center, 1 = quad
    /// edge; the classic disc edge sits at 0.46). Sizes the disc and its
    /// glow together. **A radius of 0 turns the core off** — the outer
    /// octave glyphs then carry the note alone.
    pub core_radius: f32,
    /// The outer octave layer's radial band (same UV units): every outer
    /// style fits its glyphs' radial footprint to this. derive_scene keeps
    /// outer ahead of inner, so any dragged combination still renders a
    /// visible band.
    pub outer_inner: f32,
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
    pub outer_gap: f32,
    /// How many octaves one turn of a node covers at FULL SIZE (see
    /// [`octaves`](crate::octaves)), 1..=11 — not how many it draws, which is
    /// this plus twice [`octave_extras`](Self::octave_extras). Each is exactly
    /// one octave and they all share whatever the extras leave, so this says
    /// how many degrees an octave of the main register is worth. Notes past
    /// either end of the whole wheel light the outermost indicator on their
    /// side.
    pub octave_count: u32,
    /// The MIDI pitch at the TOP of the wheel — on every node, whatever its
    /// pitch class: a node's ring is turned so that its own octaves land on
    /// their pitches, by up to half a slice either way.
    /// [`sanitize`](Self::sanitize) holds it to the settable limits.
    pub octave_center: f32,
    /// Extra octaves at EACH end of the wheel, drawn small: 0..=5, and never
    /// so many that the whole wheel passes eleven slices. Each one reaches an
    /// octave further up AND down the keyboard for a sliver of the turn, where
    /// an octave of count is paid for by every full-size octave at once.
    pub octave_extras: u32,
    /// How wide one extra is, as a fraction of an EVEN slice (the turn over
    /// the whole wheel, extras included), 0.1..=1. Under 1 an extra is always
    /// narrower than a full-size octave, whatever the count and however many
    /// extras there are, and 1 is an even wheel.
    pub octave_extra_size: f32,
    /// How much the extras GRADE from the outermost inward, 0..1: 0 is a flat
    /// fringe of equal slivers and 1 is a ramp that meets the full-size
    /// octaves in a step the size of its own. The outermost extra is the size
    /// above whatever this is, so it is a shape rather than a second
    /// strength — and it is inert without two extras to differ.
    pub octave_extra_blend: f32,
    // Which shimmer sweeps the octave glyphs has no field here: the sheet is
    // the melody/bass rings' alone (`pulse_marks`), and the octave layer draws
    // steady whatever those are doing. Saved blobs still carry the
    // `pulse_octaves` key, naming any of the six patterns `Pulse` answers to;
    // serde ignores unknown keys, so such a blob loads intact, opens on the
    // steady octave layer, and drops the key on the next save.
    // ---- Note envelope ---------------------------------------------------
    // How a note ARRIVES and how it LEAVES, for every layer of the node at
    // once. Both run on the one host-automatable Fade param, which lives in
    // [`FrameParams`]; the shape here is the other half of it, and
    // [`ViewConfig::envelope`] is where the two are put back together.
    /// How curved both ends of the note envelope are, 0..=1: 0 the straight
    /// line every layer has always faded on, 1 the sharpest curve on offer.
    /// It walks the exponent of an ease-out — see
    /// [`Envelope::approach`](harmonigraph_core::Envelope), which is also
    /// where the case for a power over an exponential is written.
    ///
    /// One number for the arrival and the departure together, which is the
    /// point of it — a curve is a house style for how things move, and a
    /// lattice that answered the keys one way and let go another would read
    /// as two instruments. It does NOT reach the trail, whose fade is a
    /// memory decaying over tens of seconds rather than a note's own
    /// envelope, and which stays deliberately linear (see
    /// [`trail`](mod@crate::trail)).
    ///
    /// A bare `#[serde(default)]`: 0 is the straight line, so a blob that
    /// predates the bar loads drawing exactly what it was saved drawing.
    pub fade_shape: f32,
    // ---- Idle (unlit) node marker ----------------------------------------
    // A minimal grey marker at each home-sheet node, drawn ALWAYS —
    // independent of both the active appearance and whether a note is
    // playing there (an active note simply composites over it). Off-sheet
    // nodes draw nothing (the grid marks them).
    /// What the idle marker is (see [`IdleMarker`]): nothing, a filled dot,
    /// or an outline circle.
    pub idle_marker: IdleMarker,
    /// The idle marker's radius (dot fill / circle) in quad UV units.
    /// Independent of the active `core_radius`.
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
    pub mark_melody: bool,
    /// Mark the lowest held note. See [`mark_melody`](Self::mark_melody).
    pub mark_bass: bool,
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
    pub mark_thickness: f32,
    /// How long a note must HOLD an end before its ring begins to ease in,
    /// in seconds. The wait sits in front of the ease rather than stretching
    /// it: the ring is at 0 for this long and then arrives on the same Fade
    /// ramp (see [`envelope`](Self::envelope)) every other layer arrives on.
    ///
    /// The wait is also a THRESHOLD: an end that changes hands again before
    /// the delay is up never draws a ring at all. That is what the setting is
    /// for. Playing fast, the top and bottom of what is down change every few
    /// notes, and a ring easing in on each of them reads as flicker over the
    /// octave band rather than as the line it is tracing — so the delay is
    /// how long a note has to be the melody before it counts as the melody.
    ///
    /// A ring outlives its key (it fades out on the note's release), so the
    /// threshold is answered AT the key-up — `derive_scene`'s `ease` — and
    /// only the ramp runs on from there. Left to the ramp alone, an end
    /// dropped mid-delay would climb past the threshold while the note was
    /// already fading and ring a note that never was the melody, which is the
    /// very flicker this setting buys off.
    ///
    /// Not derived from the note Fade, which is the other end of the same
    /// note and reads as the natural pair: a fade is how long a note takes to
    /// LEAVE, and tying the two would mean a long release could not be paired
    /// with a ring that answers immediately. The delay is measured from the
    /// handoff the tracker stamps ([`HeldEnd`](harmonigraph_core::HeldEnd)),
    /// which is exactly why that stamp cannot come off the released voice:
    /// any delay past the Fade would outlive the note that handed the end
    /// over.
    ///
    /// 0 is the ring arriving with its note, and is deliberately not what a
    /// fresh view opens on — see `impl Default`, where the wait that stops
    /// the chord-release smear is written out.
    pub mark_delay: f32,
    /// Which shimmer sweeps the melody/bass rings (see [`Pulse`]): the sheet
    /// takes both rings AND the octave slice each one points at. That reach
    /// into the other layer is the mark being the ring together with the slice
    /// it names, so light crossing one crosses the other — and it is the whole
    /// of where the sheet goes, the octave layer drawing steady everywhere a
    /// ring does not point. [`Pulse::Off`] is both the fresh-view and the
    /// old-blob fallback, so a bare `#[serde(default)]` covers it.
    pub pulse_marks: Pulse,

    // ---- Shimmer ---------------------------------------------------------
    // The sweep's knobs: what the pattern above is sized and paced by, one
    // sheet of light crossing the whole lattice.
    //
    // All four are inert while the pattern is Off.
    /// How fast the shimmer travels, in world units per second — the
    /// lattice's own units, so the DAW window and an exported video sweep at
    /// the same rate across the same nodes, where a rate in screen pixels
    /// would not. Which WAY it travels is the pattern's own: along the bands'
    /// normal for the gratings, outward from the origin for
    /// [`Pulse::Rings`]. 0 freezes the sheet where it stands, which is a look
    /// rather than an off switch (the mode is the switch).
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
    pub shimmer_width: f32,
    /// How strong the sweep is where it passes, 0..1 being none to the full
    /// tuned depth: ONE number drives both of what a band does — how much
    /// brightness it adds to the layer, and how far the layer's coverage dips
    /// between bands — so the two can never be dialed against each other into
    /// a shimmer that brightens without dimming or the reverse.
    ///
    /// The light is ADDED rather than mixed toward white (`SHIMMER_LIFT` in
    /// `lattice.wgsl`), which is what brings one setting close to meaning one
    /// thing across the pitch ramp: a peak lands within a quarter as much on a
    /// low note's dark color as on a high note's bright one, where a mix toward
    /// a fixed white lifts whichever is further from it by twice as much and
    /// spends the color of both getting there.
    ///
    /// 0 is the layer drawing exactly as it does unshimmered, from a bar rather
    /// than from the mode. The two stop moving together well before the top,
    /// and not because they are separate: the added light CLIPS, first in
    /// whichever channel is already highest, so the ramp's most saturated color
    /// starts flattening at about 0.39 and all but its darkest by 1, while the
    /// trough goes on deepening to the clamp under every one of them. So the
    /// upper half of the bar buys its contrast by darkening the layer between
    /// bands rather than by lighting them evenly, and what it takes on the way
    /// is chroma and a little hue at the ramp's bright end — 15 degrees of it
    /// by 1. Still more shimmer, and worth having, but a different trade from
    /// the bottom half.
    ///
    /// What the light costs is real at any setting, and it is the point of
    /// the bar: under a strong band an indicator says "an octave sounds here"
    /// without saying which.
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
    pub grid_color: [f32; 4],
    /// Grid line thickness as a multiple of the built-in width. 1 is the
    /// classic hairline; the shader scales its grid half-width by this.
    pub grid_thickness: f32,
    /// How far a grid segment stops short of each node center, as a factor
    /// of the node radius — the padding between a line end and the
    /// dot/circle drawn there. 0 runs the lines right into the centers;
    /// 1.05 sits slightly wider than the disc's visual radius, so the gap
    /// fully contains a sounding note's circle.
    ///
    pub grid_inset: f32,
    /// Draw the in-plane grid lines dashed. The sevens-axis links are
    /// always dashed regardless — that dash is what distinguishes a depth
    /// link from an in-sheet line, and isn't a style choice.
    pub grid_dashed: bool,
    // ---- Trail (where the music has already been) ------------------------
    // The one part of the view about the past rather than the present. It
    // rides the IDLE layer only -- see the `trail` module for why that is
    // the whole design and not an implementation detail.
    /// How a node the music has visited is marked (see [`TrailMark`]). Off
    /// by default: showing only what is audible is what every saved view
    /// was drawn with, and leaving marks behind is a deliberate choice.
    pub trail_mark: TrailMark,
    /// How far the mark departs from a plain idle node, 0..1 — how much
    /// lighter the Lift grey, how visible the Ring, how much color the
    /// Tint. 1 is still quiet by construction; every mark is bounded well
    /// short of reading as a sounding note.
    pub trail_strength: f32,
    /// Seconds before a pitch is forgotten, measured from when it last
    /// sounded. **0 means never** -- the default, and the point of the
    /// feature: a whole piece's territory rather than a rolling window.
    pub trail_memory: f32,
    /// Keep the note name and cents on a visited node, not just on sounding
    /// and hovered ones -- so the harmonic space can be read off the screen
    /// by name, with its tuning. Independent of the mark above: the text is
    /// its own channel and is useful with the marks off.
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
    /// On by default: a project at 12-TET (400 = 4·700 − 2400) is meantone
    /// whether or not anyone said so, and its E and E- name one pitch, so
    /// the detect has something to say about most tunings without being
    /// asked. Switching this off leaves the mode wherever it is and hands
    /// the switch back.
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
    pub marvel: bool,
    /// Auto-detect marvel: [`Self::meantone_auto`]'s twin, engage-only for
    /// the same reason — the lock has to survive dragging the fifth or the
    /// third, either of which moves the derived seventh out from under a
    /// seventh param that is inert while the lock holds.
    ///
    /// On by default, on the same grounds as the meantone detect: 12-TET
    /// tempers 225/224 out as well (1000 = 2·700 + 2·400 − 1200), so a
    /// project there has one pitch under `B♭↓` and `A♯` whether or not
    /// anyone said "marvel", and the detect respelling the sevens sheet is
    /// the tuning's own arithmetic showing up in the names.
    pub marvel_auto: bool,
    /// Hide every tab bar so adjacent panes — lattice above spectrum, in the
    /// default layout — record as one seamless surface. Tab toggles it.
    ///
    /// The separators keep their regular width, so the spacing between panes
    /// is the same in both modes and a take framed in one is framed in the
    /// other.
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
    pub show_perf: bool,
    /// Expand the overlay from the headline numbers into the full per-stage
    /// breakdown of where a frame goes.
    ///
    /// Off by default: the breakdown exists to answer "which stage is eating
    /// the frame", and once it has, a dozen rows of scaffolding is not what
    /// you want sitting over the picture. Inert while `show_perf` is off.
    pub show_perf_detail: bool,
    /// Offscreen render resolution as a multiple of the pane's native pixel
    /// size: >1 supersamples (crisper glyph edges), <1 renders coarse and
    /// upscales. 1.0 reproduces the pre-offscreen-pass output exactly.
    pub render_scale: f32,
    /// Bloom post-process: how much blurred brightness gets added back
    /// as a halo around bright notes. 0 disables the chain entirely — the
    /// composite is then exactly the plain scene, so there is deliberately
    /// no separate on/off toggle.
    pub bloom_strength: f32,
}

impl ViewConfig {
    /// The note envelope, assembled from the two halves it is stored in: the
    /// shape is a LOOK and lives here, the duration is host-automatable and
    /// lives in [`FrameParams`] — one Fade param for both the arrival and the
    /// departure, so a note comes up on the same time it goes down on.
    ///
    /// One assembly point, called by everything that needs an envelope, so
    /// the split is invisible past this line and no caller can pair a
    /// duration with the wrong shape. The shape is clamped to the range its
    /// bar offers — a hand-edited blob can hold anything, and `sanitize` only
    /// repairs the non-finite (a finite 40 would be a curve no bar can undo).
    pub fn envelope(&self, frame: &FrameParams) -> Envelope {
        Envelope {
            attack_time: frame.fade_time,
            fade_time: frame.fade_time,
            shape: self.fade_shape.clamp(0.0, 1.0),
        }
    }

    /// Whether a melody/bass ring can be drawn at all: an end has to be
    /// marked for there to BE a ring, and the thickness has to leave it
    /// something to draw with.
    ///
    /// One predicate because three places have to agree on it — the pane
    /// grays the ring's own controls, `derive_scene` folds
    /// [`pulse_marks`](Self::pulse_marks) off, and the Shimmer bars gray on
    /// the same thing, that pattern being the only thing they size — and three
    /// copies of a two-term condition is how they come to disagree. They did:
    /// the fold tested the thickness alone, so a saved view with both marks off
    /// kept shipping a live pulse mode, and the Shimmer bars tested neither and
    /// stayed draggable with nothing to drag.
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

    /// Fit a deserialized view to what its controls can actually produce.
    ///
    /// A bar cannot produce a nonsense value but a hand-edited RON can, and
    /// these feed a rasterizer.
    ///
    /// Most repairs fall back to the fresh view's own value, which is the only
    /// other value in the file known to be drawable. `fade_shape` and
    /// `mark_delay` are the exception and land on 0 — both have a 0 that
    /// MEANS something (straight, no wait), so a blob that has lost the
    /// number gets the inert setting rather than a look nobody asked for. It
    /// reads as a feature switched off, which is what a lost number should
    /// look like.
    pub fn sanitize(&mut self) {
        let fresh = ViewConfig::default();

        // Fit the label scale to what its bar offers. It multiplies a FONT
        // SIZE, and the bar cannot produce a nonsense value where a
        // hand-edited blob can: a non-finite one reaches egui as a glyph with
        // no image, so every label silently vanishes, and a huge one asks the
        // rasterizer for a glyph wider than the texture atlas can hold.
        self.label_scale = if self.label_scale.is_finite() {
            self.label_scale.clamp(0.3, 3.0)
        } else {
            fresh.label_scale
        };

        // The pitch gradient's four knobs, for the same reason as the label
        // scale above and one more: they are the memo key of the color table
        // every pitch-colored shape reads, so a non-finite one would miss the
        // cache on every lookup as well as drawing a NaN.
        self.pitch_gradient = self.pitch_gradient.sanitized();

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
            fresh.octave_extra_size
        };
        self.octave_extra_blend = if self.octave_extra_blend.is_finite() {
            self.octave_extra_blend.clamp(0.0, 1.0)
        } else {
            fresh.octave_extra_blend
        };

        // The shimmer's four knobs, on the same grounds and against the same
        // hole in `clamp`. `derive_scene` clamps all four into their ranges
        // every frame, which is what the shader trusts — and a NaN walks
        // through a clamp untouched, because every comparison against it is
        // false. From there it is a divide (the period), a `pow` exponent
        // (the softness) and two mixes, so ONE non-finite number in a
        // hand-edited blob NaNs the sheet, and a NaN sheet takes the rings and
        // the slices they name with it wherever the mode is on. Repaired here rather
        // than in `derive_scene` because this is the blob's own door: the bars
        // cannot reach these values, so a view that holds one got it from a
        // file.
        // The mark delay, against that same hole: it is added to a timestamp
        // and the sum divided by the attack, so a non-finite one poisons the
        // ease of every ring. The symptom is the rings VANISHING, not drawing
        // wrong — a NaN level fails `Mark::add`'s `>=` (NaN answers no to
        // every comparison), so the level stays at 0 while the slot bit is
        // still set, and the shader multiplies the ring's coverage away to
        // nothing. Silent, and it takes the whole layer wherever the marks
        // are on. `derive_scene` clamps the RANGE (the bar cannot leave it; a
        // file can), which is again no guard against a NaN.
        self.mark_delay = finite_or(self.mark_delay, 0.0);

        // The envelope shape, against the same hole. `Envelope::approach`
        // guards its own arithmetic against a non-finite duration or shape —
        // it has to, being reachable from a shell that never went through
        // this door — but it guards by treating the transition as already
        // OVER, so a NaN shape would show as a curve that silently
        // straightens. The picture quietly drawing something other than what
        // the bar reads out is exactly what this door is for. The duration
        // itself is the host-automatable Fade param, not a blob field, so it
        // has no door here to guard.
        self.fade_shape = finite_or(self.fade_shape, 0.0);

        self.shimmer_speed = finite_or(self.shimmer_speed, fresh.shimmer_speed);
        self.shimmer_width = finite_or(self.shimmer_width, fresh.shimmer_width);
        self.shimmer_intensity = finite_or(self.shimmer_intensity, fresh.shimmer_intensity);
        self.shimmer_softness = finite_or(self.shimmer_softness, fresh.shimmer_softness);
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

/// The look a fresh view starts in, and the single source of every field's
/// fallback: the container-level `#[serde(default)]` on the struct means a
/// blob missing a key picks its value up from here.
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
            // Reach and fade all but equal, which puts the clearing at full
            // strength to about the node's rim and gone a bit over a quarter
            // of a node-width past it.
            sevens_gutter: 0.278_196_75,
            sevens_gutter_soft: 0.271_047_7,
            sevens_label: SevensLabel::Name,
            show_labels: true,
            // Labels at their built-in size.
            label_scale: 1.0,
            show_cents: true,
            // Written out rather than taken from `PitchGradient::default()`,
            // which is the gradient TYPE's own default — the CIELAB arc
            // converted, which `the_defaults_are_the_retired_arc_converted`
            // holds it to, and what a gradient assembled in code opens on.
            // The composed look is free to differ, and does: a shorter arc,
            // a dimmer middle over a shallower brightness ramp, and a little
            // more chroma.
            pitch_gradient: PitchGradient {
                hue_start: 257.842_65,
                hue_span: 190.0,
                lightness: 53.0,
                lightness_ramp: 31.0,
                chroma: 0.601_670_8,
            },
            // A small, soft core inside the octave band, with the band's
            // silent slots ghosted in: the pitch class reads as a compact
            // center and the octaves carry the node's outline. (The band's
            // own width is set below.)
            core_solidity: 0.4,
            core_radius: 0.256_256_76,
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
            octave_extras: 2,
            octave_extra_size: 0.387_534_47,
            octave_extra_blend: 0.562_241_4,
            // The top of the bar: the sharpest ease on offer, both ends
            // arriving and leaving fast and settling slowly rather than
            // sliding out at one rate.
            fade_shape: 1.0,
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
            // Thin rings, slit at the marked octave's boundaries.
            mark_thickness: 0.078_269_88,
            // Just past a passing sixteenth — 125ms at 120bpm — which is
            // where the wait stops rejecting notes anyone is listening to and
            // starts rejecting the ones nobody is. Not the 0 the bar's low
            // end offers, because a ring outlives its key (see `mark_delay`):
            // at 0 every momentary crowning rings its way OUT over the whole
            // Fade, so lifting a chord one key at a time leaves a fading ring
            // on nearly every note of it, and opening there would ship the
            // mechanism switched off.
            mark_delay: 0.15,
            pulse_marks: Pulse::Bands,
            // The sheet the rings above wear. A period well under one node's
            // spacing puts several of them across every ring, so this reads as
            // a fine texture ON the marks rather than as light crossing the
            // lattice — which is what keeps it off the reading of the octave
            // slice each ring points at, the one place the sheet touches the
            // glyph layer. Half depth and a slow pace hold it there; wider
            // and deeper it would be a sweep crossing the lattice instead.
            shimmer_speed: 0.335_761_5,
            shimmer_width: 0.639_271_56,
            shimmer_intensity: 0.517_033_16,
            shimmer_softness: 1.0,
            // The grid's color comes from the skin, which is the only place
            // it comes from until someone customizes it.
            grid_color: skin::active_skin().grid_line.to_array(),
            grid_thickness: 1.103_806_3,
            grid_inset: 0.3,
            grid_dashed: false,
            // Trail on, with the note names kept on visited nodes. NOTE:
            // Lift works by lightening the idle marker, and the marker is
            // None above, so out of the box the trail shows as the labels
            // alone. See TrailMark::needs_idle_marker.
            trail_mark: TrailMark::Lift,
            // Half travel on a bar whose whole range is quiet: clearly a
            // different node, still clearly not a lit one.
            trail_strength: 0.5,
            trail_memory: 0.0,
            trail_labels: true,
            meantone: false,
            meantone_auto: true,
            marvel: false,
            marvel_auto: true,
            frameless: false,
            show_perf: false,
            show_perf_detail: false,
            render_scale: 1.0,
            // A halo at about four fifths strength: the small soft core and
            // the thin octave marks are quiet shapes, and the bloom is what
            // gives them presence.
            bloom_strength: 0.806_154_85,
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
    /// Seconds a note takes to arrive, and once released, to leave — one
    /// time for EVERY layer of the node: the pitch class core, the octave
    /// glyphs, and the melody/bass marks. One time rather than one per layer
    /// or one per direction, so an arrival or a release reads as a single
    /// gesture instead of layers moving at different moments or rates.
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
