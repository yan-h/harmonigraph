//! Per-frame scene derivation: turns the note tracker + tuning into the
//! node/edge instance lists the renderer draws. Animation and envelope
//! *policy* lives here.

use crate::camera::Camera;
use crate::color::{channel_color, idle_color, pitch_ramp_lut};
use crate::style::HighlightExtremes;
use crate::trail::TrailField;
use crate::view::{FrameParams, ViewConfig};
use crate::{
    lattice_to_world, EdgeInstance, NodeInstance, Scene, NODE_RADIUS_FACTOR,
    OCTAVE_ATTACK_TIME, OCTAVE_SLOTS,
};
use glam::Vec4;
use lattice_core::{ChannelRole, LatticePos, NoteTracker, Tuning};

/// Identifies one voice, matching `NoteTracker`'s own held-voice key.
type VoiceKey = (u8, u8);

/// A melody- or bass-ring accumulator for one node: which octave slots it
/// marks, plus the color and envelope of the strongest marking voice seen so
/// far. The `>=` in [`Mark::add`] means ties favor the later voice, so a
/// release crossfading two voices lands on the newer color.
#[derive(Default)]
struct Mark {
    slots: u32,
    level: f32,
    color: Vec4,
}

impl Mark {
    fn add(&mut self, slot: usize, envelope: f32, color: Vec4) {
        self.slots |= 1 << slot;
        if envelope >= self.level {
            self.level = envelope;
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
    let node_idle = idle_color(view);
    let live_extremes = held_extremes(tracker, view.highlight_extremes);
    // Sanitized once, outside the node loop. Capped at 1: this axis makes
    // off-sheet nodes SMALLER, never larger, so the home sheet stays the
    // biggest thing on screen (see `ViewConfig::sevens_size`). The floor
    // keeps a sheet from collapsing to an invisible speck at extent 4.
    let sevens_size = view.sevens_size.clamp(0.15, 1.0);
    // Bounded well inside the billboard: the quad reaches QUAD_MARGIN (1.6)
    // in uv, and the gutter has to finish inside it or it would be clipped
    // square instead of ending as a circle.
    let sevens_gutter = view.sevens_gutter.clamp(0.0, 0.5);

    // Each voice's color, computed once here rather than re-running the
    // LCH->sRGB conversion on every node the voice matches. It depends only on
    // the voice and the frame's gradient range — never on the node — so this
    // lifts the transcendental color math out of the O(nodes × voices) loop
    // below. The melody/bass ring reuses this SAME color: the pitch ramp
    // already bakes in the lightening the disc, roll, and octave glyphs share
    // (see `color::NOTE_LIGHTEN`), so a ring must not lift it a second time or
    // it sits a shade whiter than the very note it marks.
    let voices: Vec<(&lattice_core::Voice, Vec4)> = tracker
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
                    // The mark takes the marked note's OWN color — the very one
                    // its disc and octave glyph use, so the ring reads as that
                    // exact note. The ramp is already lightened, so there is no
                    // extra lift here. Strongest marking voice wins the color;
                    // the slots still collect every one of them, since a
                    // release crossfades two.
                    if is_melody {
                        melody.add(slot, envelope, voice_color);
                    }
                    if is_bass {
                        bass.add(slot, envelope, voice_color);
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
        // decides how small it draws, whether it clears a gutter, and
        // whether it carries a comma. Distance, not signed depth — the home
        // sheet is the ground, and a sheet in front of it is no more the
        // subject than one behind (see `ViewConfig::sevens_size`).
        let sheets = centered.sevens.unsigned_abs();
        let (scale, gutter, comma) = if sheets == 0 {
            (1.0, 0.0, 0.0)
        } else {
            let scale = sevens_size.powi(sheets as i32);
            // The node this one shares a spelling with, on the home sheet:
            // the letter walk uses `threes - 2*sevens`, so undoing the
            // sevens term two fifths at a time lands on the same name.
            let namesake =
                LatticePos::new(pos.threes - 2 * centered.sevens, pos.fives, center.sevens);
            // The WIDTH of the gutter, which is a constant of the view. Its
            // STRENGTH is the note's own envelope, applied in the shader
            // against the same `activation` it paints the node with, so the
            // clearing fades out exactly as the note it belongs to does.
            //
            // Those are deliberately not the same knob. Scaling the width by
            // the envelope instead — which is what this line used to do —
            // leaves the clearing FULLY opaque for the whole release and
            // only narrows its soft edge, so the hole hangs around at full
            // strength and then vanishes the instant the voice is pruned.
            //
            // Zeroed while nothing sounds so a silent node punches nothing:
            // a node drawing only a faint trail mark, or nothing at all,
            // clearing a full-size hole in the home sheet is a hole with no
            // note in it — and, since every position matching the pitch
            // class lights, a lattice full of them.
            let gutter = if activation > 0.0 { sevens_gutter } else { 0.0 };
            (scale, gutter, wrapped_cents(node_pc, tuning.pitch_class(namesake)))
        };
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
            cents: node_pc.to_cents(),
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
        node_idle,
        trail_mark: view.trail_mark,
        trail_strength: view.trail_strength.clamp(0.0, 1.0),
        mark_unlinked: view.mark_unlinked.clamp(0.0, 1.0),
        mark_thickness: view.mark_thickness.clamp(0.0, 0.4),
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
fn wrapped_cents(from: lattice_core::PitchClass, to: lattice_core::PitchClass) -> f32 {
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
/// from a sounding off-sheet note down to the home sheet — so a note on
/// another sheet hangs from something visible instead of floating. That is
/// about ONE note's depth, not a relationship between two: in-plane lines
/// no longer brighten because the notes at both ends happen to sound.
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
                    if let Some(n) = node_at(LatticePos::new(p.threes, p.fives, s)) {
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
            // Inset each end by ITS OWN node's size: a sevens chain runs
            // between sheets that draw at different sizes, and one inset for
            // both ends would leave the small end ringed by a gap far wider
            // than the node it clears.
            let dir = (neighbor.world_pos - node.world_pos).normalize_or_zero();
            grid.push(EdgeInstance {
                a: node.world_pos + dir * inset * node.scale,
                b: neighbor.world_pos - dir * inset * neighbor.scale,
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

