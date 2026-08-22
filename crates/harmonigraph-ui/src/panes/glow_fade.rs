//! The node glow's own clock: how a node's light follows the node.
//!
//! Every drawn layer of a node rides the note Fade — the disc, the octave band,
//! the marks, the audio ring — and the light is deliberately not one of them.
//! A halo is the slow part of the picture: stepped with the marks' and the
//! ring's own fast envelopes it flickers with them, and what a light should do
//! instead is arrive a little behind the note and leave a long way behind it.
//! So it has a pair of its own ([`ViewConfig::glow_attack`] and `_release`) and
//! this is where they are spent.
//!
//! Not a pane, though it sits with them, and the same shape as its neighbour
//! [`spectral_fold`](super::spectral_fold): a post-pass the Lattice pane runs
//! over a derived scene, with the state that has to outlive a frame kept in
//! [`SharedState`] — which is exactly what the offline renderer carries between
//! frames, so a light carried anywhere else would draw one picture live and
//! another in an export.
//!
//! ## Two halves of one light
//!
//! A node's light has a LEVEL and a COLOUR, and they are carried in different
//! places because they live in different places:
//!
//! - The **level** is a number per node, and a node's identity is a
//!   [`LatticePos`]. It is stepped here.
//! - The **colour** is the node's own ink read round the node, which lives on
//!   the GPU as a row of the ink strip (`InkStrip` in harmonigraph-render) and
//!   is never mirrored on the CPU — the layer colours are stated in WGSL by the
//!   same functions that draw them, and a second spelling in Rust is the one
//!   thing the design does not want. It is stepped in the shader, against the
//!   row it left there last frame.
//!
//! What ties the two is [`GlowStep::mix`], the coefficient this pass computes:
//! the level takes it here and the strip takes the same one there, so the two
//! halves of one light cannot come to run at different speeds.
//!
//! ## Rows, and why they are handed out rather than counted
//!
//! The strip's rows used to be the instance buffer's own order, which is fine
//! for a picture assembled from one frame and useless for one carried between
//! them: the instance list is sorted by depth and culled, so a node's index in
//! it moves under it. A row that moves is a node reading a stranger's ink and
//! morphing toward that.
//!
//! So a row is HANDED OUT when a node's light starts and handed back when it
//! ends, and [`GlowFade`] is the map. Rows are recycled, and the strip's height
//! is the high-water mark of how many have been out at once, rounded up to a
//! power of two — it grows and never shrinks, because shrinking it rebuilds the
//! textures and takes every node's colour history with it. Growing does too;
//! rounding up is what makes that rare, and the frame it happens on is a
//! [`GlowStep::mix`] of 1 everywhere, which seeds rather than fading up from a
//! texture nobody drew.

use std::collections::HashMap;

use harmonigraph_core::LatticePos;
use harmonigraph_scene::{GlowStep, Scene, ViewConfig};

use crate::spectrum::hop_alpha;
use crate::SharedState;

/// Below this a node's light is over: it is dropped to exactly 0, the node
/// stops being drawn into the strip, and its row goes back.
///
/// A release approaches 0 without reaching it, so something has to say when a
/// light has ended — the alternative is a row per node ever played, held for
/// the session against a level nothing can see. At the Strength bar's own 1
/// this is 0.4/255 of light at the hottest pixel of the halo, which is under
/// half a step of the 8-bit target it is composited into: the cut is invisible
/// wherever it lands, and it lands a little over six release constants after
/// the note (`ln(1/0.002)`).
const GONE: f32 = 0.002;

/// The fewest rows the strip is ever built for, and the floor the doubling
/// starts from.
///
/// The pair of strips is 8 bytes a texel, so 64 rows is 66 KB — nothing beside
/// the pane's own offscreen, and enough that an ordinary lattice never grows
/// the texture at all.
const FIRST_ROWS: u32 = 64;

/// The most nodes that can be lit at once.
///
/// A ceiling on the TEXTURE rather than on the picture: a strip is one row per
/// lit node and a texture has a maximum dimension (8192 on every backend the
/// plugin runs on), where [`MAX_DRAWN_NODES`](harmonigraph_scene::MAX_DRAWN_NODES)
/// is 20480 — a fully zoomed-out lattice with the audio ring's Gate at its
/// floor lights every node in the window, so the two numbers do meet. Past this
/// a node simply gets no light: its layers draw exactly as they do with the
/// glow off, which is the graceful end of a halo nobody can pick out of four
/// thousand others anyway.
const MAX_ROWS: u32 = 4096;

/// Where every node's light has got to, and which row of the ink strip is
/// keeping its colour.
///
/// One of these per lattice SURFACE, because the strip it describes is per
/// surface: the docked pane and the Video tab's preview draw their own windows
/// into their own textures, and one map shared between them would hand a row to
/// a node only one of them can see.
///
/// Stepped against a moment rather than a duration ([`at`](Self::at)), like
/// [`RingFade`](harmonigraph_scene::RingFade) beside it: a surface drawn twice
/// at one clock moves nothing the second time.
#[derive(Clone, Default)]
pub struct GlowFade {
    /// Every node with a light, by the identity that outlives a frame.
    nodes: HashMap<LatticePos, Lit>,
    /// Rows whose node's light has ended, ready to be handed out again.
    free: Vec<u32>,
    /// How many rows have ever been out at once — the high-water mark the
    /// strip's height is rounded up from, and the next row to hand out when
    /// [`free`](Self::free) is empty.
    handed: u32,
    /// How tall the strip was asked to be last step, which is what says whether
    /// the textures behind it have just been rebuilt.
    rows: u32,
    /// Which step this is, so the pass can tell the nodes it has just seen from
    /// the ones that have left the window.
    frame: u64,
    /// The clock the levels stand at, or `None` before the first step.
    at: Option<f64>,
}

/// One node's light: where it has got to, and where its colour is kept.
#[derive(Clone, Copy)]
struct Lit {
    level: f32,
    row: u32,
    /// The step this node was last seen on (see [`GlowFade::frame`]).
    seen: u64,
}

/// Carry every node's light toward what its layers say now, and hand each node
/// the row its colour is kept in.
///
/// Runs AFTER [`spectral_fold::apply`](super::spectral_fold), and that order is
/// the whole reason it is a pass of its own: the audio ring is one of the
/// layers whose ink lights a node, and what the ring says at a node is not
/// known until the fold has said it.
///
/// With the light off nothing is stepped and nothing is allocated — the state
/// is dropped instead, so the frame the Reach bar comes back off 0 seeds rather
/// than fading up out of levels nobody has seen since.
pub(crate) fn apply(scene: &mut Scene, state: &mut SharedState, surface: usize, now: f64) {
    // The renderer's own test for whether the light draws at all, asked of the
    // scene's clamped copies so the two cannot disagree about the boundary.
    if scene.glow_reach <= 0.0 || scene.glow_strength <= 0.0 {
        state.glow_fade.remove(&surface);
        return;
    }
    state
        .glow_fade
        .entry(surface)
        .or_default()
        .step(scene, &state.view, now);
}

impl GlowFade {
    /// One step of the filter over `scene`: every node's own
    /// [`glow`](harmonigraph_scene::NodeInstance::glow), and
    /// [`Scene::glow_rows`] for the frame.
    ///
    /// The first step of all SETTLES rather than fading in, which falls out of
    /// the arithmetic rather than needing a case: with no clock behind it the
    /// step is infinitely long and the coefficient is 1. So is a node's first
    /// frame, and for the same reason one step further on — there is no picture
    /// to transition from, and a light fading up from a row nobody drew is a
    /// node arriving out of black.
    fn step(&mut self, scene: &mut Scene, view: &ViewConfig, now: f64) {
        let dt = self.at.map_or(f64::INFINITY, |at| now - at);
        self.at = Some(now);
        self.frame += 1;
        // Two coefficients for the whole frame: every node's light runs on the
        // same pair of times, and which of the two it takes is which way it is
        // going.
        let (up, down) = (hop_alpha(view.glow_attack, dt), hop_alpha(view.glow_release, dt));
        // The ring is a layer of the node exactly while the layer draws.
        // `audio_ring` is 1 on a scene nothing has measured (see
        // `NodeInstance::audio_ring`), so asking it unconditionally would light
        // every drawn node at full on a view whose ring is off.
        let ringing = scene.spectral.ring_draws();
        for node in &mut scene.nodes {
            // The largest level that puts ink on this node — every layer the
            // light is assembled out of, since a node with a mark or a ring and
            // no key down is a node with something on screen.
            let target = node
                .activation
                .max(node.melody_level)
                .max(node.bass_level)
                .max(if ringing { node.audio_ring } else { 0.0 })
                .clamp(0.0, 1.0);
            node.glow = match self.nodes.get_mut(&node.lattice_pos) {
                Some(lit) => {
                    lit.seen = self.frame;
                    let mix = if target > lit.level { up } else { down };
                    lit.level += (target - lit.level) * mix;
                    // The end of a light, said exactly rather than approached:
                    // a level that only gets small keeps a row and an instance
                    // for the rest of the session. Only where there is nothing
                    // left to follow — a target that is small but real is a
                    // node still drawing something, and a light cut off under
                    // one would be handed the same row back next frame.
                    if lit.level < GONE && target <= 0.0 {
                        lit.level = 0.0;
                    }
                    GlowStep { level: lit.level, row: lit.row, mix }
                }
                // A node with no light yet and none arriving is left alone: a
                // row handed to a target of 0 would be handed straight back.
                None if target <= 0.0 => GlowStep::default(),
                None => match self.free.pop().or_else(|| {
                    (self.handed < MAX_ROWS).then(|| {
                        self.handed += 1;
                        self.handed - 1
                    })
                }) {
                    Some(row) => {
                        self.nodes.insert(
                            node.lattice_pos,
                            Lit { level: target, row, seen: self.frame },
                        );
                        // Settled, not faded in: this node's row holds whatever
                        // the last node to own it left there.
                        GlowStep { level: target, row, mix: 1.0 }
                    }
                    // Every row is spoken for, so this node has no light at
                    // all — see `MAX_ROWS`.
                    None => GlowStep::default(),
                },
            };
        }
        // Every node whose light this frame did not see has left the window, so
        // there is nothing to carry and nothing to draw. Its row goes back.
        let (frame, free) = (self.frame, &mut self.free);
        self.nodes.retain(|_, lit| {
            let kept = lit.seen == frame && lit.level > 0.0;
            if !kept {
                free.push(lit.row);
            }
            kept
        });
        let rows = self.handed.next_power_of_two().max(FIRST_ROWS);
        // A strip of a different height is a strip built afresh, so every row
        // in it holds nothing and every node has to seed. The level is left
        // where the filter put it — a light that jumped to its target here
        // would pop on the one frame the texture happened to grow.
        //
        // Against what this pass asked for last time and not against the
        // scene's own field, which `derive_scene` fills afresh every frame with
        // a row per node.
        if rows != self.rows {
            for node in &mut scene.nodes {
                node.glow.mix = 1.0;
            }
            self.rows = rows;
        }
        scene.glow_rows = rows;
    }
}

#[cfg(test)]
mod tests {
    use harmonigraph_core::{LatticePos, NoteEvent};

    use super::*;
    use crate::tests::probe::fresh;

    /// The lattice the pane derives, at `now`.
    fn scene_at(state: &SharedState, now: f64) -> Scene {
        harmonigraph_scene::derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.view.reach(),
            &state.frame_params,
            state.camera,
            None,
            now,
        )
    }

    /// One node of a scene, by the identity the filter keys on.
    fn node_at(scene: &Scene, pos: LatticePos) -> harmonigraph_scene::NodeInstance {
        *scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == pos)
            .expect("the window holds this node")
    }

    /// A state with the light on and both of its times named, and the note
    /// Fade at nothing so what a layer does is a step and what the LIGHT does
    /// is the only thing being filtered.
    fn lit(attack: f32, release: f32) -> SharedState {
        let mut state = fresh();
        state.view.glow_reach = 0.8;
        state.view.glow_attack = attack;
        state.view.glow_release = release;
        state.frame_params.fade_time = 0.0;
        state
    }

    /// A node whose every layer has gone silent keeps its light, and loses it
    /// over the Glow release rather than with the note.
    ///
    /// The whole point of the pair: a halo that left with its note is one more
    /// layer of the node, and reads as one. Measured with the note Fade at 0,
    /// so the node's own ink stops in ONE frame and everything left on screen
    /// afterwards is the light's own.
    #[test]
    fn a_nodes_light_outlives_every_layer_that_lit_it() {
        const TAU: f64 = 0.5;
        let mut state = lit(0.0, TAU as f32);
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        let mut fade = GlowFade::default();
        let mut scene = scene_at(&state, 0.0);
        fade.step(&mut scene, &state.view, 0.0);
        let held = node_at(&scene, LatticePos::ORIGIN).glow.level;
        assert!(held > 0.99, "a held note lights its node at once: {held}");

        // The key comes up, and with no Fade under it the node draws nothing
        // at all from the next frame on.
        state.tracker.handle_event(NoteEvent::off(0.0, 0, 60));
        let level = |fade: &mut GlowFade, state: &SharedState, now: f64| {
            let mut scene = scene_at(state, now);
            fade.step(&mut scene, &state.view, now);
            let node = node_at(&scene, LatticePos::ORIGIN);
            assert_eq!(node.activation, 0.0, "the note itself must be gone by {now}s");
            node.glow.level
        };
        let just_after = level(&mut fade, &state, 0.05);
        assert!(just_after > 0.5, "the light left with the note: {just_after}");
        let one_tau = level(&mut fade, &state, TAU);
        assert!(one_tau < just_after, "the light is not leaving: {one_tau} then {just_after}");
        assert!(
            (one_tau - just_after / std::f32::consts::E).abs() < 0.05,
            "a time constant on should leave 1/e of it, and left {one_tau} of {just_after}",
        );
        // And it ends, rather than approaching nothing for the rest of the
        // session — with the row it was holding.
        let over = level(&mut fade, &state, 7.0 * TAU);
        assert_eq!(over, 0.0, "the light never ended: {over}");
        assert!(fade.nodes.is_empty(), "a light that is over still holds a row");
    }

    /// A node's row holds still while its neighbour comes and goes.
    ///
    /// The row is where last frame's colour is read back from, so a row that
    /// moved under a node would be that node taking on whatever ink the node
    /// that used to own it left there. What moves it, if anything does, is the
    /// instance list — sorted by depth and culled, so a neighbour arriving
    /// renumbers everything after it.
    #[test]
    fn a_nodes_row_holds_still_while_its_neighbour_comes_and_goes() {
        let mut state = lit(0.0, 0.05);
        // A fifth up is one step along the threes axis, which is a node of its
        // own on the same sheet.
        let neighbour = LatticePos::new(1, 0, 0);
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        let mut fade = GlowFade::default();
        let mut rows = Vec::new();
        let step = |fade: &mut GlowFade, state: &SharedState, now: f64| {
            let mut scene = scene_at(state, now);
            fade.step(&mut scene, &state.view, now);
            let held = node_at(&scene, LatticePos::ORIGIN).glow;
            (held.row, node_at(&scene, neighbour).glow)
        };
        rows.push(step(&mut fade, &state, 0.0));
        // The neighbour arrives...
        state.tracker.handle_event(NoteEvent::on(0.1, 0, 67, 1.0));
        rows.push(step(&mut fade, &state, 0.1));
        rows.push(step(&mut fade, &state, 0.2));
        // ...and leaves, long enough ago for its light to be over.
        state.tracker.handle_event(NoteEvent::off(0.2, 0, 67));
        rows.push(step(&mut fade, &state, 1.0));
        assert!(
            rows[1].1.level > 0.0 && rows[2].1.level > 0.0,
            "the neighbour never lit: {rows:?}",
        );
        assert_eq!(rows[3].1.level, 0.0, "the neighbour's light never ended: {rows:?}");
        let held: Vec<u32> = rows.iter().map(|(row, _)| *row).collect();
        assert!(
            held.iter().all(|row| *row == held[0]),
            "the held node's row moved under it: {held:?}",
        );
        // And the neighbour's row came back rather than being lost. Its row
        // while it was lit, which is the one the light that ended was holding.
        let handed_back = rows[2].1.row;
        assert!(
            !fade.nodes.contains_key(&neighbour),
            "a node whose light is over is still in the map",
        );
        assert!(
            fade.free.contains(&handed_back),
            "row {handed_back} was not handed back: {:?}",
            fade.free,
        );
    }

    /// The light off is the light not stepped: no state, and every node's step
    /// left exactly as `derive_scene` handed it over.
    #[test]
    fn a_view_with_no_light_carries_nothing() {
        let mut state = lit(0.3, 2.5);
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        let mut scene = scene_at(&state, 0.0);
        let before = node_at(&scene, LatticePos::ORIGIN).glow;
        state.view.glow_reach = 0.0;
        scene.glow_reach = 0.0;
        apply(&mut scene, &mut state, 0, 0.0);
        assert_eq!(node_at(&scene, LatticePos::ORIGIN).glow, before);
        assert!(state.glow_fade.is_empty(), "a light that is off kept state");
    }
}
