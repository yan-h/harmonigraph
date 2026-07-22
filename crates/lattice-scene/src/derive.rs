//! Per-frame scene derivation: turns the note tracker + tuning into the
//! node/edge instance lists the renderer draws. Animation and envelope
//! *policy* lives here.

use crate::camera::Camera;
use crate::color::{channel_color, idle_color, pitch_ramp_lut};
use crate::style::HighlightExtremes;
use crate::trail::TrailField;
use crate::view::{FrameParams, ViewConfig};
use crate::{
    lattice_to_world, EdgeInstance, NodeInstance, Scene, MARK_WHITEN, NODE_RADIUS_FACTOR,
    OCTAVE_ATTACK_TIME, OCTAVE_SLOTS,
};
use glam::Vec4;
use lattice_core::{ChannelRole, LatticePos, NoteTracker, Tuning};

/// Identifies one voice, matching `NoteTracker`'s own held-voice key.
type VoiceKey = (u8, u8);

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
    which: HighlightExtremes,
) -> (Option<VoiceKey>, Option<VoiceKey>) {
    if which == HighlightExtremes::Off {
        return (None, None);
    }
    let held = || {
        tracker
            .voices()
            .filter(|v| v.state == lattice_core::VoiceState::Held)
    };
    let key = |v: &lattice_core::Voice| (v.channel, v.note);
    let melody = which
        .marks_melody()
        .then(|| held().max_by(|a, b| a.pitch.total_cmp(&b.pitch)).map(key))
        .flatten();
    let bass = which
        .marks_bass()
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
    voice: &lattice_core::Voice,
    live: (Option<VoiceKey>, Option<VoiceKey>),
) -> (bool, bool) {
    match voice.state {
        // `live` is already filtered by `which` (see held_extremes).
        lattice_core::VoiceState::Held => {
            let key = Some((voice.channel, voice.note));
            (live.0 == key, live.1 == key)
        }
        lattice_core::VoiceState::Released { .. } => (false, false),
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
    let live_extremes = held_extremes(tracker, view.highlight_extremes);

    for pos in view.visible_positions() {
        let node_pc = tuning.pitch_class(pos);

        let mut activation = 0.0f32;
        let mut octaves = [0f32; OCTAVE_SLOTS];
        let mut color = idle_color(view);
        let mut outlined = false;
        let mut seed = 0.0f32;
        let mut melody_slots = 0u32;
        let mut bass_slots = 0u32;
        let mut melody_level = 0.0f32;
        let mut bass_level = 0.0f32;
        let mut melody_color = Vec4::ZERO;
        let mut bass_color = Vec4::ZERO;

        // O(nodes × voices); fine at this scale. If extents grow large,
        // index voices by quantized pitch class instead.
        for voice in tracker.voices() {
            if tuning.matches(voice.pitch_class, node_pc) {
                let envelope = voice.activation(now, frame.fade_time);
                if envelope > activation {
                    activation = envelope;
                    color = channel_color(
                        voice.channel,
                        voice.pitch,
                        frame.darkest_pitch,
                        frame.brightest_pitch,
                    );
                    outlined = ChannelRole::of(voice.channel) == ChannelRole::Outline;
                    seed = (voice.on_time % 256.0) as f32;
                }
                let slot = voice.octave.clamp(0, OCTAVE_SLOTS as i8 - 1) as usize;
                // Smoothstep ease-in over the first OCTAVE_ATTACK_TIME;
                // release still fades on the octave envelope.
                let t = ((now - voice.on_time) / OCTAVE_ATTACK_TIME).clamp(0.0, 1.0) as f32;
                let attack = t * t * (3.0 - 2.0 * t);
                octaves[slot] = octaves[slot].max(envelope * attack);

                // Mark the outer notes in the slot they sound in. Set on
                // every node the voice matches, exactly as its activation
                // is, so the mark can't disagree with the lighting.
                //
                // The level is the strongest marking voice ON THIS NODE.
                // Only held voices mark, so the ring is full while its end is
                // held and gone the frame it is released — it never outlives
                // the key, even as the disc keeps fading.
                let (is_melody, is_bass) = marks(voice, live_extremes);
                if is_melody || is_bass {
                    // The mark takes the marked note's OWN color, lightened
                    // so a low note's near-black doesn't vanish. Strongest
                    // marking voice wins the color; the slots still collect
                    // every one of them, since a release crossfades two.
                    let own = channel_color(
                        voice.channel,
                        voice.pitch,
                        frame.darkest_pitch,
                        frame.brightest_pitch,
                    )
                    .lerp(Vec4::ONE, MARK_WHITEN);
                    if is_melody {
                        melody_slots |= 1 << slot;
                        if envelope >= melody_level {
                            melody_level = envelope;
                            melody_color = own;
                        }
                    }
                    if is_bass {
                        bass_slots |= 1 << slot;
                        if envelope >= bass_level {
                            bass_level = envelope;
                            bass_color = own;
                        }
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
            cents: node_pc.to_cents(),
            melody_slots,
            bass_slots,
            melody_level,
            bass_level,
            melody_color,
            bass_color,
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
    let outer_solidity = view.outer_solidity.clamp(0.0, 1.0);
    // A gap wider than the band would erase the sectors entirely; cap it
    // well short of that.
    let outer_gap = view.outer_gap.clamp(0.0, 0.4);
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
        outer_gap,
        idle_marker: view.idle_marker,
        idle_radius: view.idle_radius.clamp(0.0, 0.9),
        grid,
        grid_thickness: view.grid_thickness.clamp(0.0, 8.0),
        node_idle: idle_color(view),
        trail_mark: view.trail_mark,
        trail_strength: view.trail_strength.clamp(0.0, 1.0),
        mark_unlinked: view.mark_unlinked.clamp(0.0, 1.0),
        mark_thickness: view.mark_thickness.clamp(0.0, 0.4),
        pitch_lut: pitch_ramp_lut(),
        darkest_pitch: frame.darkest_pitch,
        brightest_pitch: frame.brightest_pitch,
        render_scale: view.render_scale,
        bloom_strength: view.bloom_strength,
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
/// from a sounding off-sheet note down to the home sheet — so a note on
/// another sheet hangs from something visible instead of floating. That is
/// about ONE note's depth, not a relationship between two: in-plane lines
/// no longer brighten because the notes at both ends happen to sound.
pub(crate) fn derive_grid(view: &ViewConfig, nodes: &[NodeInstance]) -> Vec<EdgeInstance> {
    let inset = view.spacing * NODE_RADIUS_FACTOR * view.grid_inset.max(0.0);
    let base = Vec4::from_array(view.grid_color);
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

            // A sevens link lights as part of the chain from any sounding
            // node beyond it (away from the home sheet) down to the home
            // sheet, so an off-sheet note always hangs from a visible chain
            // even while the notes under it are silent. Nothing else
            // lights.
            let mut lit = 0.0f32;
            let mut lit_color = Vec4::ZERO;
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
            grid.push(EdgeInstance {
                a: node.world_pos + dir * inset,
                b: neighbor.world_pos - dir * inset,
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
pub(crate) fn node_seed(pos: LatticePos) -> f32 {
    let h = pos
        .threes
        .wrapping_mul(731)
        .wrapping_add(pos.fives.wrapping_mul(2683))
        .wrapping_add(pos.sevens.wrapping_mul(9461));
    (h as f32 * 0.618_034).rem_euclid(256.0)
}

