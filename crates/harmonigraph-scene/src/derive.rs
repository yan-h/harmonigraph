//! Per-frame scene derivation: turns the note tracker + tuning into the
//! node/edge instance lists the renderer draws. Animation and envelope
//! *policy* lives here.

use crate::camera::Camera;
use crate::color::{channel_color, idle_color, pitch_lut_color, pitch_ramp_lut};
use crate::octaves::octave_layout;
use crate::trail::TrailField;
use crate::view::{FrameParams, ViewConfig};
use crate::{
    lattice_to_world, EdgeInstance, NodeInstance, Scene, ATTACK_TIME,
    NODE_RADIUS_FACTOR, OCTAVE_SLOTS,
};
use glam::Vec4;
use harmonigraph_core::{ChannelRole, LatticePos, NoteTracker, Time, Tuning, VoiceState};

/// Identifies one voice, matching `NoteTracker`'s own held-voice key.
type VoiceKey = (u8, u8);

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

/// The highest and lowest HELD voices, as the caller asked for them —
/// either is `None` when that end isn't being marked or nothing is held.
///
/// Held only: the melody/bass rings live on the notes actually down. A
/// released voice is no longer part of the chord and wears no mark at all —
/// its ring comes off the instant the key does, even as its disc fades out
/// (see [`marks`]). This is what keeps a chord's release from smearing a
/// fading melody/bass ring across every note as the keys lift one by one.
///
/// Compared on `pitch` rather than the raw key, because MPE and per-note
/// tuning can bend a voice past its neighbor — the same reason the notes
/// pane sorts on pitch.
pub(crate) fn held_extremes(
    tracker: &NoteTracker,
    mark_melody: bool,
    mark_bass: bool,
) -> (Option<VoiceKey>, Option<VoiceKey>) {
    if !mark_melody && !mark_bass {
        return (None, None);
    }
    let held = || {
        tracker
            .voices()
            .filter(|v| v.state == harmonigraph_core::VoiceState::Held)
    };
    let key = |v: &harmonigraph_core::Voice| (v.channel, v.note);
    let melody = mark_melody
        .then(|| held().max_by(|a, b| a.pitch.total_cmp(&b.pitch)).map(key))
        .flatten();
    let bass = mark_bass
        .then(|| held().min_by(|a, b| a.pitch.total_cmp(&b.pitch)).map(key))
        .flatten();
    (melody, bass)
}

/// Which ends `voice` currently wears, as `(melody, bass)`. Only a HELD
/// voice wears an end — the melody/bass rings live on the notes actually
/// down. A released voice wears neither: its ring comes off the instant the
/// key does, even though its disc keeps fading. Both can be true at once —
/// a lone held note is its own melody and bass.
fn marks(
    voice: &harmonigraph_core::Voice,
    live: (Option<VoiceKey>, Option<VoiceKey>),
) -> (bool, bool) {
    match voice.state {
        // `live` is already filtered by `which` (see held_extremes).
        harmonigraph_core::VoiceState::Held => {
            let key = Some((voice.channel, voice.note));
            (live.0 == key, live.1 == key)
        }
        harmonigraph_core::VoiceState::Released { .. } => (false, false),
    }
}

/// How far an indicator that arrived at `since` has eased in, 0..1:
/// smoothstep over the first [`ATTACK_TIME`] seconds. Shared by the octave
/// sectors and the melody/bass rings, so a note's whole outer layer arrives
/// as one gesture.
fn attack(now: Time, since: Time) -> f32 {
    let t = ((now - since) / ATTACK_TIME).clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

/// When the voice keyed `key` TOOK the end it now wears — the instant its
/// ring should start easing in. Its own note-on, unless it INHERITED the
/// end from a note that has since come off: then the moment that note was
/// released, which is when this one became the outer voice. `outranks` says
/// which direction beats it (a higher note for the melody, a lower one for
/// the bass).
///
/// Without the inherited case, lifting the top of a held chord would drop
/// the melody ring onto the note below at full strength in a single frame —
/// the ring has moved, but the note it moved to is old, so its note-on is
/// long past and there is nothing left to ease.
///
/// Read off the tracker rather than remembered between frames: a released
/// voice sticks around for the whole of its fade, which outlasts this ramp
/// unless the Fade param is turned down near zero — and at that setting
/// nothing else eases either.
fn end_taken_at(
    tracker: &NoteTracker,
    key: Option<VoiceKey>,
    outranks: fn(f32, f32) -> bool,
) -> Option<Time> {
    let key = key?;
    let voice = tracker.voices().find(|v| (v.channel, v.note) == key)?;
    let mut taken = voice.on_time;
    for other in tracker.voices() {
        if let VoiceState::Released { at } = other.state {
            if outranks(other.pitch, voice.pitch) && at > taken {
                taken = at;
            }
        }
    }
    Some(taken)
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
    // Each ring's ease-in, resolved once per frame: which note wears an end
    // and how long it has worn it are properties of the CHORD, identical on
    // every node the note lights.
    let melody_attack = end_taken_at(tracker, live_extremes.0, |other, own| other > own)
        .map_or(1.0, |taken| attack(now, taken));
    let bass_attack = end_taken_at(tracker, live_extremes.1, |other, own| other < own)
        .map_or(1.0, |taken| attack(now, taken));
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
        view.octave_taper_amount,
        view.octave_taper_shape,
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
        let mut seed = 0.0f32;
        let mut melody = Mark::default();
        let mut bass = Mark::default();

        // O(nodes × voices); fine at this scale. If extents grow large,
        // index voices by quantized pitch class instead.
        for &(voice, voice_color) in &voices {
            if tuning.matches(voice.pitch_class, node_pc) {
                let envelope = voice.activation(now, frame.fade_time);
                if envelope > activation {
                    activation = envelope;
                    color = voice_color;
                    outlined = ChannelRole::of(voice.channel) == ChannelRole::Outline;
                    seed = (voice.on_time % 256.0) as f32;
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
                // Eases in from note-on; release still fades on the octave
                // envelope.
                octaves[slot] = octaves[slot].max(envelope * attack(now, voice.on_time));

                // Mark the outer notes in the slot they sound in. Set on
                // every node the voice matches, exactly as its activation
                // is, so the mark can't disagree with the lighting.
                //
                // The level is the strongest marking voice ON THIS NODE.
                // Only held voices mark, so the ring eases in while its end
                // is held and is gone the frame it is released — it never
                // outlives the key, even as the disc keeps fading.
                let (is_melody, is_bass) = marks(voice, live_extremes);
                if is_melody || is_bass {
                    // The mark takes the color of the SECTOR it links back to
                    // — the pitch of that slot on this node, through the very
                    // table the shader tints the lit glyph from — so the ring
                    // is never a shade off the one indicator it is pointing
                    // at. The voice's own color is the wrong one to reuse
                    // here: a note past either end of the window folds onto
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
                    // on purpose. The ramp is already lightened, so there is
                    // no extra lift here.
                    //
                    // Strongest marking voice wins the color; the slots still
                    // collect every one of them, since a release crossfades
                    // two. The ring eases in on the SAME ramp as that sector
                    // — from when the note took the end, which is not always
                    // its note-on (see `end_taken_at`).
                    let mark_color = pitch_lut_color(
                        octave_layout.slot_pitch(slot as i32, node_cents),
                        frame.darkest_pitch,
                        frame.brightest_pitch,
                    );
                    if is_melody {
                        melody.add(slot, envelope * melody_attack, mark_color);
                    }
                    if is_bass {
                        bass.add(slot, envelope * bass_attack, mark_color);
                    }
                }
            }
        }

        // Field styles animate one continuous field per node — global time
        // as the clock plus a stable per-node seed — so pressing,
        // retriggering, or stacking notes lights the flow up without ever
        // restarting or reshuffling it. Steady ignores the seed entirely,
        // so its per-note value below is written and never read.
        let seed = if view.node_style.is_field_style() { node_seed(pos) } else { seed };

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
            seed,
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

    Scene {
        nodes,
        camera,
        time: (now % 3600.0) as f32,
        node_radius: view.spacing * NODE_RADIUS_FACTOR,
        node_style: view.node_style,
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
        mark_thickness: view.mark_thickness.clamp(0.0, 0.4),
        sevens_soft: view.sevens_gutter_soft.clamp(0.0, 0.5),
        background: crate::skin::panel_color(),
        pitch_lut: pitch_ramp_lut(),
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

/// Stable per-node animation seed for the field styles: a hash of the
/// lattice position folded into the same small range as the per-note
/// seed. A node's gas pattern becomes part of its identity — the same
/// every time it lights, decorrelated from its neighbors'.
pub(crate) fn node_seed(pos: LatticePos) -> f32 {
    let h = pos
        .threes
        .wrapping_mul(731)
        .wrapping_add(pos.fives.wrapping_mul(2683))
        .wrapping_add(pos.sevens.wrapping_mul(9461));
    (h as f32 * 0.618_034).rem_euclid(256.0)
}

