//! Per-frame scene derivation: turns the note tracker + tuning into the
//! node/edge instance lists the renderer draws. Animation and envelope
//! *policy* lives here.

use crate::camera::Camera;
use crate::color::{channel_color, idle_color, pitch_lut_color, pitch_ramp_lut};
use crate::octaves::octave_layout;
use crate::trail::TrailField;
use crate::view::{FrameParams, ViewConfig};
use crate::{
    lattice_to_world, EdgeInstance, NodeInstance, Pulse, Scene,
    MARK_DELAY_MAX, NODE_RADIUS_FACTOR, OCTAVE_SLOTS,
};
use glam::Vec4;
use harmonigraph_core::{ChannelRole, HeldEnd, LatticePos, NoteTracker, Time, Tuning, VoiceState};

/// A melody- or bass-ring accumulator for one node: which octave slots it
/// marks, plus the color and drawn level of the strongest marking voice seen
/// so far — the voice's envelope times how far its ring has eased in. The
/// `>=` in [`Mark::add`] means ties favor the later voice, so a release
/// crossfading two voices lands on the newer color.
#[derive(Default)]
struct Mark {
    slots: u32,
    level: f32,
    color: Vec4,
}

impl Mark {
    fn add(&mut self, slot: usize, level: f32, color: Vec4) {
        self.slots |= 1 << slot;
        if level >= self.level {
            self.level = level;
            self.color = color;
        }
    }
}

/// The highest and lowest HELD voices with the moment each took its end, as
/// the caller asked for them — either is `None` when that end isn't being
/// marked or nothing is held.
///
/// Held only, which is the tracker's own answer ([`HeldEnd`]): these are the
/// LIVE ends, the notes actually down, and a released voice is never one of
/// them however brightly its ring is still drawn. What a released voice wears
/// is the stamp it left with
/// ([`Voice::wore_high`](harmonigraph_core::Voice::wore_high)), read in
/// [`marks`] — a ring on its way out, not a claim on the end.
///
/// The two must not be conflated: a released voice allowed back in here would
/// keep the end from the note that replaced it, and the incoming ring would
/// have nothing to ease from.
pub(crate) fn held_extremes(
    tracker: &NoteTracker,
    mark_melody: bool,
    mark_bass: bool,
) -> (Option<HeldEnd>, Option<HeldEnd>) {
    (
        mark_melody.then(|| tracker.highest_held()).flatten(),
        mark_bass.then(|| tracker.lowest_held()).flatten(),
    )
}

/// Which ends `voice` wears and WHEN it took each, as `(melody, bass)` —
/// `None` where it wears that end not at all. Both can be set at once: a lone
/// note is its own melody and bass.
///
/// A held voice wears the live end. A released one wears the stamp it left
/// the held set with, so its ring fades out with the note instead of snapping
/// off at the key — which is the same envelope every other layer of the node
/// leaves on, and the reason a handoff reads as one ring crossing to another
/// rather than as one vanishing and a second appearing.
///
/// The released stamps are gated on the same two flags here that
/// [`held_extremes`] applies to the live ends. They have to be: a stamp is
/// left whatever the view says, so a ring turned off mid-fade would otherwise
/// go on drawing until the note it belongs to is pruned.
fn marks(
    voice: &harmonigraph_core::Voice,
    live: (Option<HeldEnd>, Option<HeldEnd>),
    mark_melody: bool,
    mark_bass: bool,
) -> (Option<Time>, Option<Time>) {
    match voice.state {
        // `live` is already filtered by those same flags (see held_extremes).
        harmonigraph_core::VoiceState::Held => {
            let key = (voice.channel, voice.note);
            let wears =
                |end: Option<HeldEnd>| end.filter(|end| end.key == key).map(|end| end.since);
            (wears(live.0), wears(live.1))
        }
        harmonigraph_core::VoiceState::Released { .. } => (
            voice.wore_high.filter(|_| mark_melody),
            voice.wore_low.filter(|_| mark_bass),
        ),
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
    // Kept parallel to `nodes` for the trail, which matches remembered
    // pitches against every node afterwards and would otherwise have to
    // recompute each node's pitch class to do it.
    let mut node_pcs = Vec::with_capacity(view.visible_count());
    let center = view.center();
    let node_idle = idle_color(view);
    let live_extremes = held_extremes(tracker, view.mark_melody, view.mark_bass);
    let mark_delay = view.mark_delay.clamp(0.0, MARK_DELAY_MAX) as f64;
    let env = view.envelope(frame);
    // How far a ring taken at `since` has eased in, for the voice `state`.
    //
    // The delay simply moves the ramp's start later, so an end held for less
    // than it never draws a ring at all. That is what the setting is for: at
    // speed the ends change hands every few frames, and a ring easing in on
    // each of them reads as flicker rather than as the top line.
    //
    // A RELEASED voice reads its ease at the moment its key came up rather
    // than at `now`, which is what keeps that threshold a threshold. A ring
    // fades out with its note, so its ease outlives the key — left running,
    // an end dropped halfway through the delay would keep climbing after the
    // release and a ring would appear on a note that never rang while it was
    // down, which is exactly the flicker the delay exists to prevent. Frozen,
    // a ring leaves at the level it actually reached: nothing if it never
    // earned one.
    //
    // The disc does the opposite — its attack keeps climbing past the key, so
    // a staccato note peaks after the note-off (see `Envelope::attack`). The
    // two differ because the disc's ramp is only about smoothing an arrival,
    // while this one is also a rule about which notes count.
    let ease = |since: Time, state: VoiceState| {
        let read_at = match state {
            VoiceState::Held => now,
            VoiceState::Released { at } => at,
        };
        env.attack(read_at, since + mark_delay)
    };
    // Sanitized once, outside the node loop. Capped at 1: this axis makes
    // off-sheet nodes SMALLER, never larger, so the home sheet stays the
    // biggest thing on screen (see `ViewConfig::sevens_size`). The floor
    // keeps a sheet from collapsing to an invisible speck at extent 4.
    let sevens_size = view.sevens_size.clamp(0.15, 1.0);
    // Bounded well inside the billboard: the quad reaches QUAD_MARGIN (1.6)
    // in uv, and the gutter has to finish inside it or it would be clipped
    // square instead of ending as a circle.
    let sevens_gutter = view.sevens_gutter.clamp(0.0, 0.5);
    // The octave wheel is a pitch axis, so it is a property of the VIEW and is
    // built once: every node draws the same slice WIDTHS. Which octaves those
    // slices are, and how far the ring is turned to put them on their pitches,
    // is per node — see the fold below.
    let octave_layout = octave_layout(
        view.octave_count,
        view.octave_center,
        view.octave_extras,
        view.octave_extra_size,
        view.octave_extra_blend,
    );

    // Each voice's color, computed once here rather than re-running the
    // LCH->sRGB conversion on every node the voice matches. It depends only on
    // the voice and the frame's gradient range — never on the node — so this
    // lifts the transcendental color math out of the O(nodes × voices) loop
    // below. The melody/bass rings do NOT reuse it — they belong to the octave
    // layer, which is colored by axis position (see the mark color below).
    let voices: Vec<(&harmonigraph_core::Voice, Vec4)> = tracker
        .voices()
        .map(|voice| {
            let color = channel_color(
                voice.channel,
                voice.pitch,
                frame.darkest_pitch,
                frame.brightest_pitch,
                view.pitch_gradient,
            );
            (voice, color)
        })
        .collect();

    for pos in view.visible_positions() {
        let node_pc = tuning.pitch_class(pos);
        let node_cents = node_pc.to_cents();
        // The octaves of THIS pitch class nearest the center pitch, which is
        // what its indicators are. Per node rather than per frame: where a
        // class falls against the center decides which octaves of it are the
        // nearest ones, and the ring is turned to match.
        let (low_slot, high_slot) = octave_layout.slots(node_cents);
        let mut activation = 0.0f32;
        let mut octaves = [0f32; OCTAVE_SLOTS];
        let mut color = node_idle;
        let mut outlined = false;
        let mut melody = Mark::default();
        let mut bass = Mark::default();

        // O(nodes × voices); fine at this scale. If extents grow large,
        // index voices by quantized pitch class instead.
        for &(voice, voice_color) in &voices {
            if tuning.matches(voice.pitch_class, node_pc) {
                let envelope = voice.activation(now, &env);
                if envelope > activation {
                    activation = envelope;
                    color = voice_color;
                    outlined = ChannelRole::of(voice.channel) == ChannelRole::Outline;
                }
                // The slot whose own pitch on THIS node is the one sounding —
                // `slot_pitch` solved for the slot, which is what keeps the
                // indicator that lights the one the note is drawn at. Taking
                // the voice's MIDI octave instead is the same number whenever
                // the two pitch classes sit on the same side of the octave,
                // and one out when they straddle it: `matches` wraps, so a
                // node a shade under 1200¢ is lit by a played 0¢, and its
                // pitches are an octave below what the voice's own octave
                // names. Middle C on an untransposed lattice is slot 5 either
                // way.
                //
                // Clamped into the octaves the ring draws: a note past either
                // end lights the outermost indicator on its side rather than
                // vanishing, which is what keeps a narrow span a way of
                // READING the music rather than a filter over it.
                //
                // And then into the packing, which is the MIDI octaves and
                // nothing else: a ring near the pitch limits draws octaves no
                // note can reach (see `Ring::base`), and the outermost
                // indicator a note can fold onto is the outermost one that has
                // a slot at all. The two ranges always overlap — a ring is at
                // least two octaves wide and its middle is a playable pitch —
                // so this lands on a slice that is drawn.
                let sounding = ((voice.pitch - node_cents / 100.0) / 12.0).round() as i32;
                let slot = sounding
                    .clamp(low_slot, high_slot)
                    .clamp(0, OCTAVE_SLOTS as i32 - 1) as usize;
                // The voice's envelope entire — the attack is already in it
                // ([`Voice::activation`]), and the release still fades on the
                // octave's own voice rather than on the node's.
                octaves[slot] = octaves[slot].max(envelope);

                // Mark the outer notes in the slot they sound in. Set on
                // every node the voice matches, exactly as its activation
                // is, so the mark can't disagree with the lighting.
                //
                // The level is the strongest marking voice ON THIS NODE. A
                // ring eases in while its end is held and fades out with the
                // note when the key comes up, on the note's own release — so
                // a handoff is one ring crossing to another rather than one
                // vanishing as a second appears.
                let (melody_since, bass_since) =
                    marks(voice, live_extremes, view.mark_melody, view.mark_bass);
                if melody_since.is_some() || bass_since.is_some() {
                    // The mark takes the color of the SECTOR it links back to
                    // — the pitch of that slot on this node, through the very
                    // table the shader tints the lit glyph from — so the ring
                    // is never a shade off the one indicator it is pointing
                    // at. The voice's own color is the wrong one to reuse
                    // here: a note past either end of the ring folds onto
                    // the outermost slot, so its ring would carry a pitch
                    // that is nowhere on the axis it is drawn around.
                    //
                    // Off the pitch ramp whatever the channel, because a LIT
                    // glyph is: the shader tints one by its own pitch and
                    // asks nothing about the voice, so a fixed-color or
                    // outline channel that keeps its hue on the disc still
                    // brackets a ramp-colored sector. Only the lit glyph —
                    // the band's ghosts wear the whitened node color, and a
                    // solo voice's glow keeps the channel hue, both of them
                    // on purpose. No extra lift on top of the ramp here: the
                    // sector's glyph wears it as it comes, and a lightened
                    // ring would read a shade off the one it brackets.
                    //
                    // Strongest marking voice wins the color; the slots still
                    // collect every one of them, since a release crossfades
                    // two. The ring eases in on the SAME ramp as that sector,
                    // from when the note took the end — which is not always
                    // its note-on, and is the tracker's own answer rather
                    // than anything derived here (`HeldEnd` while the note is
                    // down, `Voice::wore_high` once it is not).
                    let mark_color = pitch_lut_color(
                        octave_layout.slot_pitch(slot as i32, node_cents),
                        frame.darkest_pitch,
                        frame.brightest_pitch,
                        view.pitch_gradient,
                    );
                    // The RELEASE alone under the ring's own ease, not the
                    // node's full activation: the attack is in that, and the
                    // ring already carries one from the moment its note took
                    // the end. Multiplying both in would square the ramp
                    // wherever those two moments coincide — which is the
                    // ordinary case, a note arriving as the new outer voice —
                    // and a ring rising as the square of the sector it
                    // brackets is precisely the disagreement about how fast
                    // the note arrived that one shared rate exists to
                    // prevent.
                    //
                    // Per voice rather than once per frame, which the ease was
                    // when only held notes could wear a ring: a chord has one
                    // melody, but it can have several notes on their way out
                    // that each wore the end when they left, and they leave
                    // from different levels at different moments.
                    let release = voice.release_level(now, &env);
                    if let Some(since) = melody_since {
                        melody.add(slot, release * ease(since, voice.state), mark_color);
                    }
                    if let Some(since) = bass_since {
                        bass.add(slot, release * ease(since, voice.state), mark_color);
                    }
                }
            }
        }

        // World positions are relative to the window center, keeping the
        // displayed region under the camera wherever the window pans.
        let centered = pos - center;
        let world_pos = lattice_to_world(centered, view.spacing);

        // The sevens layer: how far off the home sheet this node sits
        // decides how small it draws and whether it carries a comma.
        // Distance, not signed depth — the home sheet is the ground, and a
        // sheet in front of it is no more the subject than one behind (see
        // `ViewConfig::sevens_size`).
        let sheets = centered.sevens.unsigned_abs();
        let (scale, comma) = if sheets == 0 {
            (1.0, 0.0)
        } else {
            // The node this one shares a LETTER with, on the home sheet: the
            // letter walk uses `threes - 2*sevens`, so undoing the sevens
            // term two fifths at a time lands on the same letter and
            // accidental. Not the same name — the septimal mark the name now
            // carries is exactly what separates them.
            let namesake =
                LatticePos::new(pos.threes - 2 * centered.sevens, pos.fives, center.sevens);
            (
                sevens_size.powi(sheets as i32),
                wrapped_cents(node_pc, tuning.pitch_class(namesake)),
            )
        };
        // EVERY sounding node clears what is behind it, the home sheet
        // included. Leaving the home sheet out is what let the sheets
        // behind it show straight through the gaps in a home node's body —
        // the core is small and soft and the octave band is a thin annulus,
        // so "drawn over" covers very little — and neither sheet then read
        // as being in front of the other.
        //
        // The home sheet's clearing cuts the grid lines as well as the
        // sheets behind it — a sounding node sits in a clean gap in the
        // lattice rather than on top of it. That is reason enough on its
        // own, so it does NOT wait for depth: on a flat lattice there are
        // no sheets to hide, but the grid is still there to be cut, and a
        // gap in the lines is the look either way. (It was gated on the
        // sevenths extent when the clearing was purely an inter-sheet
        // device; a flat lattice then had to turn the gutter on by growing
        // depth it didn't want.)
        //
        // The WIDTH is a constant of the view; the STRENGTH is the note's
        // envelope, applied in the shader against the same `activation` it
        // paints the node with, so a clearing fades out exactly as its note
        // does. Scaling the width by the envelope instead leaves the
        // clearing fully opaque for the whole release and only narrows its
        // soft edge, so the hole hangs around at full strength and then
        // vanishes the instant the voice is pruned.
        let gutter = if activation > 0.0 { sevens_gutter } else { 0.0 };

        nodes.push(NodeInstance {
            lattice_pos: pos,
            world_pos,
            color,
            activation,
            octaves,
            outlined,
            hovered: hovered == Some(pos),
            on_home: pos.sevens == view.center_sevens,
            scale,
            gutter,
            comma,
            cents: node_cents,
            melody_slots: melody.slots,
            bass_slots: bass.slots,
            melody_level: melody.level,
            bass_level: bass.level,
            melody_color: melody.color,
            bass_color: bass.color,
            trail: 0.0,
        });
        node_pcs.push(node_pc);
    }

    // Marks only the idle layer (see `trail`), so nothing downstream that
    // reads "is sounding" — the grid's sevens chains above all — can pick a
    // memory up by mistake, whatever order these run in.
    if let Some(field) = TrailField::build(tracker.history(), view, frame, now) {
        field.apply(&mut nodes, &node_pcs, tuning);
    }

    let grid = derive_grid(view, &nodes);

    // Core/outer geometry policy: the core is a plain radius the shader
    // reads (0 = off), with solidity riding alongside; the outer band is
    // sanitized here so the shader can trust outer > inner whatever the two
    // bars hold.
    let core_radius = view.core_radius.clamp(0.0, 0.9);
    let core_solidity = view.core_solidity.clamp(0.0, 1.0);
    let outer_inner = view.outer_inner.clamp(0.0, 0.9);
    let outer_outer = view.outer_outer.clamp(outer_inner + 0.05, 1.0);
    // A gap wider than the band would erase the sectors entirely; cap it
    // well short of that.
    let outer_gap = view.outer_gap.clamp(0.0, 0.4);
    let mark_thickness = view.mark_thickness.clamp(0.0, 0.4);

    Scene {
        nodes,
        camera,
        now,
        node_radius: view.spacing * NODE_RADIUS_FACTOR,
        core_radius,
        core_solidity,
        outer_inner,
        outer_outer,
        outer_gap,
        octave_layout,
        idle_marker: view.idle_marker,
        idle_radius: view.idle_radius.clamp(0.0, 0.9),
        grid,
        grid_thickness: view.grid_thickness.clamp(0.0, 8.0),
        node_idle,
        trail_mark: view.trail_mark,
        trail_strength: view.trail_strength.clamp(0.0, 1.0),
        mark_thickness,
        // A mark sheet reaches the ring AND the octave slice that ring points
        // at, so with the ring layer off — no end marked, or no thickness to
        // draw one with, which is `mark_rings_draw` and is exactly what the
        // pane grays the row on — the ring half is multiplied away by a
        // coverage of zero and the SLICE half is not: the glyph layer draws
        // that either way, so without this fold the sheet would go on
        // sweeping from a control the user can no longer reach. Folded here
        // so the picture and the control agree by construction.
        pulse_marks: if view.mark_rings_draw() { view.pulse_marks } else { Pulse::Off },
        shimmer_speed: view.shimmer_speed.clamp(0.0, 40.0),
        // Strictly positive: the pattern's phase divides by this. The floor is
        // a small fraction of a node (radius `spacing` × NODE_RADIUS_FACTOR),
        // not a fraction of the lattice — several periods crossing one node at
        // once is a look the bar reaches on purpose.
        shimmer_width: view.shimmer_width.clamp(0.02, 40.0),
        shimmer_intensity: view.shimmer_intensity.clamp(0.0, 4.0),
        // Unlike the three above, this one is clamped to exactly its bar,
        // because it is a SHAPE rather than an amount and the bar's ends are
        // the shape's: past 1 the shader's exponent drops below 1 and the lit
        // part widens past the dark, so the pattern reads as thin rifts in a
        // lit layer rather than as light crossing a clear one, and below 0 the
        // crest narrows away to a spike too fine for any pixel to catch.
        shimmer_softness: view.shimmer_softness.clamp(0.0, 1.0),
        sevens_soft: view.sevens_gutter_soft.clamp(0.0, 0.5),
        background: crate::skin::panel_color(),
        pitch_lut: pitch_ramp_lut(view.pitch_gradient),
        darkest_pitch: frame.darkest_pitch,
        brightest_pitch: frame.brightest_pitch,
        render_scale: view.render_scale,
        bloom_strength: view.bloom_strength,
    }
}

/// Signed cents from `to` to `from`, folded into ±600 — the short way round
/// the octave. Pitch classes wrap, so the raw difference between a node and
/// its namesake can come out an octave off and read as a 1173-cent "comma".
fn wrapped_cents(from: harmonigraph_core::PitchClass, to: harmonigraph_core::PitchClass) -> f32 {
    let d = from.to_cents() - to.to_cents();
    if d > 600.0 {
        d - 1200.0
    } else if d < -600.0 {
        d + 1200.0
    } else {
        d
    }
}

/// Line opacity of a lit sevens-axis chain link.
const GRID_LIT_OPACITY: f32 = 0.85;

/// The faint background grid: idle positions draw no disc, so these
/// segments carry the lattice's structure instead, inset at both ends so
/// each node position keeps a clear circular gap. Only the home (center)
/// sheet draws an idle grid.
///
/// The one thing that lights is a dashed sevens-axis link, as the chain
/// from a FLOATING sounding off-sheet note down to the home sheet — so a
/// note on another sheet hangs from something visible instead of floating.
/// That is about ONE note's depth, not a relationship between two: in-plane
/// lines do not brighten because the notes at both ends happen to sound.
///
/// A chain runs only through silence, and stops at the first sounding note
/// under it: a note already sitting over a sounding one is connected to it
/// visibly, by being the same site a step apart, and the line would only
/// say so twice. Lit or not, a link keeps the lattice's own color — it says
/// where a note hangs from, not what the note is.
pub(crate) fn derive_grid(view: &ViewConfig, nodes: &[NodeInstance]) -> Vec<EdgeInstance> {
    let inset = view.spacing * NODE_RADIUS_FACTOR * view.grid_inset.max(0.0);
    let base = Vec4::from_array(view.grid_color);
    // `nodes` is exactly `view.visible_positions()` in order: a dense
    // row-major grid (threes outer, fives, sevens inner). So a neighbor's
    // index is plain offset arithmetic — no per-frame HashMap build and no
    // hashing per lookup. Returns None for positions outside the window. The
    // explicit per-axis bounds are what keep an out-of-range delta from
    // aliasing onto a different node's slot.
    let min_threes = view.center_threes - view.extent_threes;
    let min_fives = view.center_fives - view.extent_fives;
    let min_sevens = view.center_sevens - view.extent_sevens;
    let span_fives = 2 * view.extent_fives + 1;
    let span_sevens = 2 * view.extent_sevens + 1;
    let node_at = |p: LatticePos| -> Option<&NodeInstance> {
        let (dt, df, ds) = (p.threes - min_threes, p.fives - min_fives, p.sevens - min_sevens);
        if dt < 0
            || df < 0
            || ds < 0
            || dt > 2 * view.extent_threes
            || df > 2 * view.extent_fives
            || ds > 2 * view.extent_sevens
        {
            return None;
        }
        nodes.get(((dt * span_fives + df) * span_sevens + ds) as usize)
    };
    // Upper bound: three +1 axis-steps per node.
    let mut grid = Vec::with_capacity(nodes.len() * 3);
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
            let Some(neighbor) = node_at(step) else {
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

            // A sevens link lights as part of the chain hanging a sounding
            // off-sheet note from something visible: it runs from that note
            // down toward the home sheet — and only through SILENCE. The
            // moment there is a sounding note under it to hang from, the
            // chain has done its job and stops.
            //
            // Drawing it anyway is what made a 7-limit note sitting over a
            // sounding one wear a dash it did not need: the two are already
            // connected, visibly, by being the same site one step apart, and
            // the line only says it a second time. It is the FLOATING note —
            // nothing sounding beneath it anywhere down the column — that
            // has no anchor without one.
            let mut lit = 0.0f32;
            if along_sevens {
                let level = |s: i32| {
                    node_at(LatticePos::new(p.threes, p.fives, s))
                        .map_or(0.0, |n| n.activation)
                };
                // `p` is the link's lower index; which of its ends is the
                // one nearer home flips with the side of the axis, and so
                // does which direction "beyond" runs.
                let (beyond, toward_home) = if p.sevens >= view.center_sevens {
                    (
                        p.sevens + 1..=view.center_sevens + view.extent_sevens,
                        view.center_sevens..=p.sevens,
                    )
                } else {
                    (
                        view.center_sevens - view.extent_sevens..=p.sevens,
                        p.sevens + 1..=view.center_sevens,
                    )
                };
                // Inclusive of the link's own near end: a note directly on
                // top of a sounding one needs no line at all.
                if !toward_home.into_iter().any(|s| level(s) > 0.0) {
                    lit = beyond.into_iter().map(level).fold(0.0f32, f32::max);
                }
            }

            // Fully invisible: skip the instance instead of shipping a
            // discarded quad.
            if idle <= 0.0 && lit <= 0.0 {
                continue;
            }
            // Inset each end by ITS OWN node's size: a sevens chain runs
            // between sheets that draw at different sizes, and one inset for
            // both ends would leave the small end ringed by a gap far wider
            // than the node it clears.
            let dir = (neighbor.world_pos - node.world_pos).normalize_or_zero();
            grid.push(EdgeInstance {
                a: node.world_pos + dir * inset * node.scale,
                b: neighbor.world_pos - dir * inset * neighbor.scale,
                // The lattice's own color, always. A lit link is the same
                // structural line as an unlit one, merely brighter — it
                // says WHERE a note hangs from, not what the note is, and
                // the note's color is already on the node at each end.
                // Taking the note's hue made the chain read as a third
                // sounding thing strung between two others.
                color: base,
                strength: idle + (GRID_LIT_OPACITY - idle) * lit,
                dashed,
            });
        }
    }
    grid
}
