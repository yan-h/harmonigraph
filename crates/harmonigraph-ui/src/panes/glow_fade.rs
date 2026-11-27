//! The node glow's own clock: how a node's light follows the node.
//!
//! Every drawn layer of a node rides the note Fade — the octave band, the
//! marks, the audio ring — and the light is deliberately not one of them.
//! A halo is the slow part of the picture: stepped with the marks' own fast
//! envelopes it flickers with them, and what a light should do instead is
//! arrive a little behind the note and leave a long way behind it. So it has a
//! pair of its own
//! ([`ViewConfig::glow_attack`](harmonigraph_scene::ViewConfig::glow_attack)
//! and `_release`) and this is where they are spent.
//!
//! Not a pane, though it sits with them, and the same shape as its neighbour
//! [`spectral_fold`](super::spectral_fold): a post-pass the Lattice pane runs
//! over a derived scene, with the state that has to outlive a frame kept in
//! [`PictureState`] — which is exactly what the offline renderer carries between
//! frames, so a light carried anywhere else would draw one picture live and
//! another in an export.
//!
//! ## The whole light is carried here
//!
//! A node's light has a LEVEL and a COLOUR. Both are carried here, keyed by the
//! node's [`LatticePos`], and stepped together each frame (#1626), every value
//! in its own direction: rising on the Glow attack, falling on the release
//! (#1784 G2).
//!
//! What is carried for the colour is not a colour. It is what the light is
//! made of ([`GlowLight`]): how much light each slot and each mark puts on the
//! node. The renderer reads those weights round the node every frame, in the
//! slot colours the uniforms say now, so a gradient, range or tuning change
//! reaches a halo in the frame it reaches the notes (#1759, Q1), and the
//! colour eases as the weights do because it is a sum over them. Nothing about
//! a light outlives a frame on the GPU, so a layout pass egui discards consumes
//! nothing the renderer needed (#730's reason for keeping the colour's clock
//! there), and a light survives whatever the renderer rebuilds under it —
//! a resize (#422) or a big chord growing the strip (#1759, Q3).

use harmonigraph_core::LatticePos;
use harmonigraph_scene::{GlowLight, GlowStep, Scene, MAX_LIGHTS};

use crate::PictureState;

// Fixed-seed, as `NodeMotion` keys the same `LatticePos` identities: nothing
// here is keyed by input an attacker chooses, and SipHash is a per-node cost
// paid every frame.
type HashMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::FixedState>;

/// Below this a node's light is over: it is dropped to exactly 0, and the node
/// stops being carried.
///
/// A release approaches 0 without reaching it, so something has to say when a
/// light has ended — the alternative is an entry per node ever played, held
/// for the session against a level nothing can see. At the Background glow
/// gain bar's own 1 this is 0.4/255 of light at the hottest pixel of the halo,
/// which is under half a step of the 8-bit target it is composited into: the
/// cut is invisible wherever it lands, and it lands a little over six release
/// constants after the note (`ln(1/0.002)`).
const GONE: f32 = 0.002;

/// Half a step of the 8-bit target the glow is composited into: a change in
/// the light smaller than this does not move a pixel, so a fade that has fallen
/// under it has nothing left to repaint for ([`GlowFade::step_drawn`]).
///
/// Measured as level times the Background glow gain, which is the shader's
/// peak without its `GLOW_BASE` of 0.8, so the window errs toward staying
/// awake a little long rather than stopping on a visible step.
pub(crate) const VISIBLE_STEP: f32 = 0.5 / 255.0;

/// Where every node's light has got to.
///
/// One of these per lattice SURFACE: the docked pane and the Video tab's
/// preview draw their own windows, and a light is carried for the window that
/// draws it.
///
/// Stepped against a moment rather than a duration ([`at`](Self::at)), like
/// [`RingFade`](harmonigraph_scene::RingFade) beside it: a surface drawn twice
/// at one clock moves nothing the second time.
#[derive(Clone, Default)]
pub struct GlowFade {
    /// Every node with a light, by the identity that outlives a frame.
    nodes: HashMap<LatticePos, Lit>,
    /// Which step this is, so the pass can tell the nodes it has just seen from
    /// the ones that have left the window.
    frame: u64,
    /// The clock the levels stand at, or `None` before the first step.
    at: Option<f64>,
}

/// One node's light: where its level and what it is made of have got to.
#[derive(Clone, Copy)]
struct Lit {
    level: f32,
    light: GlowLight,
    /// The node's [`glow_target`](harmonigraph_scene::NodeInstance::glow_target)
    /// as of the last step that saw it: what the pass after the walk keeps
    /// lights by, and what [`GlowFade::clear_released`] reads between steps.
    /// Written on every step that sees the node, and a light a step does not
    /// see is dropped.
    target: f32,
    /// The step this node was last seen on (see [`GlowFade::frame`]).
    seen: u64,
}

/// How far a light carried at `level` still has to move toward `goal`, in the
/// units a falling level is measured in: the largest distance between the two
/// lights' weights, field for field, as a share of the largest weight either
/// holds, times the level.
///
/// A share because the halo's colour is the weights normalised: a light whose
/// weights all fall together toward nothing — any release — keeps its colour
/// and is the level's own fall, while an octave joining or leaving a held node
/// moves the colour by its share of the light whatever the light's size.
fn easing(level: f32, light: &GlowLight, goal: &GlowLight) -> f32 {
    let GlowLight { l, m } = light;
    let pairs =
        || [(l, &goal.l), (m, &goal.m)].into_iter().flat_map(|(a, b)| a.iter().zip(b.iter()));
    let largest = pairs().map(|(a, b)| a.abs().max(b.abs())).fold(0.0, f32::max);
    let distance = pairs().map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    if largest > 0.0 {
        level * distance / largest
    } else {
        0.0
    }
}

/// Carry every node's light toward what its layers say now.
///
/// Runs behind [`NodeMotion::step`](harmonigraph_scene::NodeMotion::step), which is
/// what a filter needs: the light it steps toward is read off the node's
/// layers, and there is nothing to step until the frame has written them. Its
/// place in the order of the passes is that and no more — the fold ahead of it
/// settles the audio ring, which is not a light (see [`GlowFade::step_drawn`]'s
/// target).
///
/// With the light off nothing is stepped — the state is dropped instead, so
/// the frame Background glow reach comes back off 0 starts a fresh attack.
pub(crate) fn apply(scene: &mut Scene, state: &mut PictureState, surface: usize, now: f64) {
    // Whether the light draws at all, by the test the renderer asks too
    // (`ViewConfig::glow_draws`), of the scene's normalized view.
    if !scene.view.glow_draws() {
        state.surfaces.glow_fade.remove(&surface);
        return;
    }
    let glow = state.surfaces.glow_fade.entry(surface).or_default();
    let moving = glow.step_drawn(scene, now);
    state.surfaces.moving_this_frame.hold_if(crate::Reason::Lattice, moving);
}

/// The attack and release coefficients for a step from `previous` to `now`:
/// how much of the way to its target a light rising and a light falling move.
///
/// A clock that ran BACKWARDS — a host looping its transport — holds every
/// light where it is, as a frame drawn twice at one time does. A negative step
/// is not a short one: run through the exponential it gives a coefficient below
/// zero, which moves the light away from its target.
fn coefficients(now: f64, previous: Option<f64>, attack: f32, release: f32) -> (f32, f32) {
    let dt = previous.map_or(f64::INFINITY, |at| now - at);
    let alpha = |seconds: f32| {
        if dt.is_nan() || dt <= 0.0 {
            0.0
        } else if !seconds.is_finite() || seconds <= 0.0 {
            1.0
        } else {
            1.0 - (-dt / f64::from(seconds)).exp() as f32
        }
    };
    (alpha(attack), alpha(release))
}

impl GlowFade {
    pub(super) fn owns(&self, pos: LatticePos) -> bool {
        self.nodes.contains_key(&pos)
    }

    /// One step of the filter over `scene`: every node's own
    /// [`glow`](harmonigraph_scene::NodeInstance::glow), and the scene's
    /// [`glow_clock`](Scene::glow_clock).
    ///
    /// A new light starts dark when attack is nonzero, including the first
    /// frame, and its level attacks from there; what it is made of starts as
    /// the node's reading whole, since a light at no level shows none of it.
    /// A node lights only where some layer that draws holds part of its light.
    ///
    /// Answers whether some light still moves a pixel on its own after the
    /// step: the larger of its level's distance from its target and its
    /// colour's still to move ([`easing`]), times the glow gain, is over
    /// [`VISIBLE_STEP`]. What the frame loop stays awake for (`lattice` in
    /// `lib.rs`), asked on the step's own draw.
    ///
    /// Stepped on the Glow times of the scene's own view, the look it was
    /// derived from.
    fn step_drawn(&mut self, scene: &mut Scene, now: f64) -> bool {
        let view = &scene.view;
        let (up, down) = coefficients(now, self.at, view.glow_attack, view.glow_release);
        let seed = if view.glow_attack <= 0.0 {
            1.0
        } else if self.at.is_none() {
            0.0
        } else {
            up
        };
        scene.glow_clock = Some(now);
        self.at = Some(now);
        self.frame += 1;
        // What moves on its own is a level off its target, rising on the
        // attack or falling on the release, and a colour still easing toward
        // its reading — a handoff, an octave joining, a look — at any level. A
        // held note's own motion is over long before a slow Glow attack is,
        // so the light answers for its rise itself. A light settled at its
        // reading is still, so a mark held with no key down keeps the loop
        // awake only while its light first rises.
        let strength = view.glow_strength;
        // A light no layer draws is no light at all (#1836). The renderer lays
        // a slot's weight on the octave band and a mark's on the mark strip,
        // each only where that layer has width (`ink_at` in `lattice.wgsl`),
        // and a halo with no ink under it is no pixels. So a node whose light
        // sits wholly on layers that draw nothing heads for none: it starts no
        // light, and so no glow passes, and does not charge one up for a layer
        // still to come, such as a mark inside its Delay with the band at 0.
        // One that shines on some drawn layer keeps its weights whole, the
        // others' included: its level is read off every layer, and gating them
        // would move the halo of a node that does draw one.
        let (band, marks) = (scene.outer_outer > scene.outer_inner, view.marks_draw());
        let shines = |light: &GlowLight| {
            (band && light.l.iter().any(|&l| l > 0.0))
                || (marks && light.m.iter().any(|&m| m > 0.0))
        };
        let mut moving = false;
        // The lights this frame sees already, which are what a new one counts
        // against the most a surface lights at once: one carried for a node
        // that has left the window is dropped below, and holds no place.
        let mut lights =
            scene.nodes.iter().filter(|n| self.nodes.contains_key(&n.lattice_pos)).count();
        // Two coefficients for the whole frame, every node's light running on
        // the same pair of times, and each VALUE in it takes its own direction
        // (#1784 G2): the level, and every slot's and mark's weight, move on
        // the attack while below where they are heading and on the release
        // while above, as a per-channel envelope follower does. So a note
        // joining a node brings its colour in on the attack, one leaving takes
        // its colour out on the release, and in an octave handoff both happen
        // at once. A level a joining note's Fade has not yet reached keeps
        // falling on the release rather than dipping on the attack.
        for node in &mut scene.nodes {
            // What puts LIGHT on this node: the MIDI layers, since a node
            // wearing a mark with no key down is still a node with something of
            // its own on screen.
            //
            // The AUDIO RING is not among them, and that is the rule rather
            // than an omission — a halo says something is being PLAYED here,
            // where the ring says something is being heard in the room, and a
            // reading a node wears is drawn and never shone. The node answers
            // that itself (`NodeInstance::glow_light`), marks included, each
            // layer read after the Opacity mapping — not
            // `presence_with_opacity`, the presence that holds through a
            // release — and the level is read off the same weights, so the two
            // halves of one light cannot come to disagree about what lit it.
            let goal = Some(node.glow_light()).filter(shines).unwrap_or_default();
            let target = goal.level();
            node.glow = match self.nodes.get_mut(&node.lattice_pos) {
                Some(lit) => {
                    lit.seen = self.frame;
                    lit.target = target;
                    let mix = if target > lit.level { up } else { down };
                    lit.level += (target - lit.level) * mix;
                    lit.light.step(&goal, up, down);
                    // The end of a light, said exactly rather than approached:
                    // a level that only gets small keeps an entry and an
                    // instance for the rest of the session. Only where there
                    // is nothing left to follow — a target that is small but
                    // real is a node still drawing something.
                    // A light carried onto layers that no longer draw, as a
                    // dial turns its last one off, ends here too: it is no
                    // light, as one that never shone is none.
                    if (lit.level < GONE && target <= 0.0) || !shines(&lit.light) {
                        lit.level = 0.0;
                    }
                    // Asked after the end is said, so the step that ends a
                    // light asks for no frame after it.
                    let seen = (lit.level - target).abs().max(easing(lit.level, &lit.light, &goal));
                    moving |= seen * strength > VISIBLE_STEP;
                    GlowStep { level: lit.level, light: lit.light }
                }
                // A node with no light yet and none arriving is left alone, and
                // so is one past the most a surface lights at once — its
                // layers draw exactly as they do with the glow off.
                None if target <= 0.0 || lights >= MAX_LIGHTS => GlowStep::default(),
                None => {
                    let level = target * seed;
                    moving |= (target - level) * strength > VISIBLE_STEP;
                    lights += 1;
                    self.nodes.insert(
                        node.lattice_pos,
                        Lit { level, light: goal, target, seen: self.frame },
                    );
                    GlowStep { level, light: goal }
                }
            };
        }
        // Every node whose light this frame did not see has left the window, so
        // there is nothing to carry and nothing to draw.
        let frame = self.frame;
        self.nodes.retain(|_, lit| lit.seen == frame && (lit.level > 0.0 || lit.target > 0.0));
        moving
    }

    /// Drop the lights whose node had nothing left to light it as of the last
    /// step — released notes' halos still riding their release — and keep
    /// every sounding one where it is (`PictureState::clear_accumulated`).
    pub(crate) fn clear_released(&mut self) {
        self.nodes.retain(|_, lit| lit.target > 0.0);
    }
}

#[cfg(test)]
mod tests {
    use harmonigraph_core::{LatticePos, NoteEvent, SourceId};

    use super::*;
    use crate::tests::probe::fresh_picture as fresh;

    /// The lattice the pane derives, at `now`.
    fn scene_at(state: &PictureState, now: f64) -> Scene {
        let mut scene = harmonigraph_scene::derive_scene(
            &state.runtime.tracker,
            &state.runtime.tuning,
            &state.appearance.view,
            &state.appearance.view.reach(),
            state.appearance.camera,
            None,
        );
        harmonigraph_scene::NodeMotion::default().step(
            &mut scene,
            &state.runtime.tracker,
            &state.runtime.tuning,
            &harmonigraph_scene::RingFade::default(),
            now,
        );
        scene
    }

    /// One node of a scene, by the identity the filter keys on.
    fn node_at(scene: &Scene, pos: LatticePos) -> harmonigraph_scene::NodeInstance {
        *scene.nodes.iter().find(|n| n.lattice_pos == pos).expect("the window holds this node")
    }

    /// A state with the light on and both of its times named, and the note
    /// Fade and the Mark delay at nothing so what a layer does is a step and
    /// what the LIGHT does is the only thing being filtered.
    fn lit(attack: f32, release: f32) -> PictureState {
        let mut state = fresh();
        state.appearance.view.glow_reach = 0.8;
        state.appearance.view.glow_attack = attack;
        state.appearance.view.glow_release = release;
        state.appearance.view.fade_time = 0.0;
        state.appearance.view.mark_delay = 0.0;
        state
    }

    /// Velocity to Opacity over a rest of about a third, as the routes stood
    /// fresh until #1475: so a soft note's slots light short of full.
    fn routed() -> harmonigraph_scene::IntensitySettings {
        use harmonigraph_scene::IntensitySource;
        harmonigraph_scene::IntensitySettings {
            velocity: IntensitySource { opacity: Some(0.6), thickness: None },
            opacity_rest: 0.3,
            ..harmonigraph_scene::IntensitySettings::default()
        }
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
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let mut fade = GlowFade::default();
        let mut scene = scene_at(&state, 0.0);
        fade.step_drawn(&mut scene, 0.0);
        let held = node_at(&scene, LatticePos::ORIGIN).glow.level;
        assert!(held > 0.99, "a held note lights its node at once: {held}");

        // The key comes up, and with no Fade under it the node draws nothing
        // at all from the next frame on.
        state.runtime.tracker.handle_event(NoteEvent::off(0.0, SourceId::DIRECT, 0, 60));
        let level = |fade: &mut GlowFade, state: &PictureState, now: f64| {
            let mut scene = scene_at(state, now);
            fade.step_drawn(&mut scene, now);
            let node = node_at(&scene, LatticePos::ORIGIN);
            assert_eq!(node.presence_with_opacity, 0.0, "the note itself must be gone by {now}s");
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
        // session — and stops being carried.
        let over = level(&mut fade, &state, 7.0 * TAU);
        assert_eq!(over, 0.0, "the light never ended: {over}");
        assert!(fade.nodes.is_empty(), "a light that is over is still carried");
    }

    /// What a light is made of eases on the very coefficient its level takes:
    /// one clock for the halo's brightness and its colour (#1626).
    ///
    /// A soft note at velocity-to-Opacity's default, so the slot's light is a
    /// fraction and a weight that moved on the wrong coefficient, or not at
    /// all, cannot land on the level's share by accident; released with no
    /// Fade, so its target is nothing and both fall at the release.
    #[test]
    fn a_lights_weights_ease_on_the_clock_its_level_takes() {
        let mut state = lit(0.2, 1.0);
        state.appearance.view.intensity = routed();
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 0.5));
        let mut fade = GlowFade::default();
        let mut step = |state: &PictureState, now: f64| {
            let mut scene = scene_at(state, now);
            fade.step_drawn(&mut scene, now);
            let node = node_at(&scene, LatticePos::ORIGIN);
            (node.glow, node.glow_light())
        };
        step(&state, 0.0);
        let rising = step(&state, 0.1);
        let held = step(&state, 0.5).0;
        let slot = (0..harmonigraph_scene::OCTAVE_SLOTS)
            .max_by(|a, b| rising.1.l[*a].total_cmp(&rising.1.l[*b]))
            .unwrap();
        let goal = rising.1.l[slot];
        assert!(goal > 0.0 && goal < 0.99, "the fixture lights one slot softly: {goal}");
        state.runtime.tracker.handle_event(NoteEvent::off(0.5, SourceId::DIRECT, 0, 60));
        let (released, now) = step(&state, 0.8);
        assert_eq!(now.level(), 0.0, "the note must be gone by the release's first step");
        let level_share = released.level / held.level;
        let light = |step: &GlowStep| step.light.l[slot];
        let light_share = light(&released) / light(&held);
        assert!(level_share < 0.9, "the release moved nothing: {level_share}");
        assert!(
            (level_share - light_share).abs() < 1e-5,
            "the level kept {level_share} and the light {light_share}",
        );
    }

    /// Each change a halo can take eases what its light is made of from the
    /// reading before to the reading after by exactly the share the Glow
    /// release has run, `1 - exp(-t / release)` — the share a halo's colour is
    /// mixed by, since the colour is linear in these weights (the renderer's
    /// `a_stepped_light_reads_as_its_two_readings_mixed`).
    ///
    /// One change per row, each moving a different part of the light, which
    /// is checked so that no row passes for want of reaching it:
    ///
    /// - an inkless release, one pitch class in two octaves at unequal
    ///   velocities: the slots' light;
    /// - the melody handed to an unmarked middle slot, three octaves held and
    ///   the top one let go: the marks;
    /// - a soft note under velocity-to-Opacity let go: a slot's light short of
    ///   full (#1315).
    ///
    /// Settled before the change, with no attack and no Fade, so the light is
    /// its reading and every layer moves in one frame; each change leaves the
    /// level at or above its target, which is the release's side.
    #[test]
    fn each_change_a_halo_takes_eases_by_the_glow_release() {
        const RELEASE: f64 = 0.5;
        const AT: f64 = 0.5;
        let on = |note, velocity| NoteEvent::on(0.0, SourceId::DIRECT, 0, note, velocity);
        let off = |note| NoteEvent::off(AT, SourceId::DIRECT, 0, note);
        type Reach = fn(&GlowLight, &GlowLight) -> bool;
        let rows: [(&str, Vec<NoteEvent>, Vec<NoteEvent>, Reach); 3] = [
            ("inkless release", vec![on(60, 1.0), on(72, 0.5)], vec![off(60), off(72)], |a, b| {
                a.l.iter().filter(|&&l| l > 0.0).count() == 2 && b.level() == 0.0
            }),
            (
                "melody handoff",
                vec![on(60, 1.0), on(72, 1.0), on(84, 1.0)],
                vec![off(84)],
                |a, b| a.m[6] == 0.0 && b.m[6] > 0.0 && a.m[7] > 0.0 && b.m[7] == 0.0,
            ),
            ("soft release", vec![on(60, 0.3)], vec![off(60)], |a, b| {
                a.level() > 0.0 && a.level() < 0.9 && b.level() == 0.0
            }),
        ];
        for (what, before, change, reach) in rows {
            let mut state = lit(0.0, RELEASE as f32);
            state.appearance.view.mark_delay = 0.0;
            state.appearance.view.intensity = routed();
            for event in before {
                state.runtime.tracker.handle_event(event);
            }
            let mut fade = GlowFade::default();
            let mut step = |state: &PictureState, now: f64| {
                let mut scene = scene_at(state, now);
                fade.step_drawn(&mut scene, now);
                let node = node_at(&scene, LatticePos::ORIGIN);
                (node.glow.light, node.glow_light())
            };
            let frames = (0..=30).map(|k| AT + f64::from(k - 30) / 60.0);
            let (from, _) = frames.map(|now| step(&state, now)).last().unwrap();
            for event in change {
                state.runtime.tracker.handle_event(event);
            }
            let (_, to) = step(&state, AT + 1.0 / 60.0);
            assert!(reach(&from, &to), "{what}: the change did not reach its part of the light");
            for k in 2..=30 {
                let elapsed = f64::from(k) / 60.0;
                let (carried, goal) = step(&state, AT + elapsed);
                assert_eq!(goal, to, "{what}: the reading moved after the change");
                let share = 1.0 - (-elapsed / RELEASE).exp() as f32;
                let mut expected = from;
                expected.step(&to, 1.0, share);
                let fields = |light: &GlowLight| [light.l, light.m].concat();
                for (i, (got, want)) in fields(&carried).iter().zip(fields(&expected)).enumerate() {
                    assert!(
                        (got - want).abs() < 1e-5,
                        "{what}, {elapsed:.3}s on: weight {i} is {got} where {share} of the way \
                         is {want}",
                    );
                }
            }
        }
    }

    /// A light whose level holds still while what it is made of eases — an
    /// octave let go under one still held at full — keeps the frame loop awake
    /// until the colour has arrived, as a falling level does; one settled asks
    /// for nothing.
    #[test]
    fn a_light_easing_its_colour_at_a_steady_level_asks_for_repaints() {
        let mut state = lit(0.0, 0.5);
        for note in [60, 72] {
            state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, note, 1.0));
        }
        let mut fade = GlowFade::default();
        let mut level_at = |state: &PictureState, now: f64| {
            let mut scene = scene_at(state, now);
            let moving = fade.step_drawn(&mut scene, now);
            (node_at(&scene, LatticePos::ORIGIN).glow.level, moving)
        };
        let (held, settled_awake) = level_at(&state, 0.5);
        assert!(!settled_awake, "a settled light asked to be repainted");
        state.runtime.tracker.handle_event(NoteEvent::off(0.5, SourceId::DIRECT, 0, 72));
        for now in [0.52, 1.0, 1.5] {
            let (level, awake) = level_at(&state, now);
            assert_eq!(level, held, "the fixture's level must hold still");
            assert!(awake, "the halo's colour was still easing at {now}s and asked for nothing");
        }
        assert!(!level_at(&state, 6.0).1, "the colour arrived and the loop stayed awake");
    }

    /// A light rising on its Glow attack asks for frames until it has
    /// arrived, its first step included, though the held note under it has
    /// long since landed (a Fade of nothing here) and asks for none: no
    /// voice holds the loop for it.
    #[test]
    fn a_light_rising_on_its_attack_asks_for_repaints_until_it_arrives() {
        let mut state = lit(0.5, 0.5);
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let mut fade = GlowFade::default();
        let mut level_at = |state: &PictureState, now: f64| {
            let mut scene = scene_at(state, now);
            let moving = fade.step_drawn(&mut scene, now);
            (node_at(&scene, LatticePos::ORIGIN).glow.level, moving)
        };
        assert!(level_at(&state, 0.0).1, "a new light starting dark asked for nothing");
        let (rising, awake) = level_at(&state, 0.5);
        assert!(rising > 0.0 && rising < 0.9, "the fixture's light must still be rising: {rising}");
        assert!(awake, "a rising light asked for nothing");
        let (arrived, awake) = level_at(&state, 20.0);
        assert!(arrived > 0.99 && !awake, "a light at {arrived} kept the loop awake");
    }

    /// A note joining a node already lit at full level brings its colour in
    /// over the Glow attack, not the release (#1784 G2): the octave above a
    /// held C, on the same node, is most of the way into the light within
    /// three attack constants, where the release would have it a fifth of the
    /// way.
    #[test]
    fn a_note_joining_a_lit_node_brings_its_colour_in_on_the_attack() {
        const ATTACK: f64 = 0.2;
        let mut state = lit(ATTACK as f32, 2.5);
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let mut fade = GlowFade::default();
        let mut step = |state: &PictureState, now: f64| {
            let mut scene = scene_at(state, now);
            fade.step_drawn(&mut scene, now);
            let node = node_at(&scene, LatticePos::ORIGIN);
            (node.glow, node.glow_light())
        };
        for k in 0..=60 {
            step(&state, f64::from(k) * 0.05);
        }
        let (held, _) = step(&state, 3.0);
        state.runtime.tracker.handle_event(NoteEvent::on(3.0, SourceId::DIRECT, 0, 72, 1.0));
        let (_, goal) = step(&state, 3.0);
        let before = held.light;
        let joined = (0..harmonigraph_scene::OCTAVE_SLOTS)
            .find(|&s| goal.l[s] > 0.0 && before.l[s] <= 0.0)
            .expect("the octave above lights a slot of its own");
        assert!(held.level > 0.99 && goal.level() > 0.99, "the node must already be lit at full");
        let mut now = 3.0;
        while now < 3.0 + 3.0 * ATTACK {
            now += 1.0 / 144.0;
            step(&state, now);
        }
        let (glow, _) = step(&state, now);
        let share = glow.light.l[joined] / goal.l[joined];
        assert!(share > 0.9, "the joining octave is {share:.2} of the way in after three attacks");
    }

    /// A pitch class struck again in another octave while its halo still
    /// lingers brings no dip in the halo's brightness: the new slice comes in
    /// over the note Fade, so its target starts under the lingering level,
    /// and the level keeps falling no faster than the release carries it until
    /// the target climbs past it (#1784 G2, B′).
    #[test]
    fn a_restruck_pitch_class_on_a_lingering_halo_brings_no_dip() {
        const RELEASE: f64 = 2.5;
        let mut state = lit(0.05, RELEASE as f32);
        state.appearance.view.fade_time = 0.3;
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let mut fade = GlowFade::default();
        let mut level = |state: &PictureState, now: f64| {
            let mut scene = scene_at(state, now);
            fade.step_drawn(&mut scene, now);
            node_at(&scene, LatticePos::ORIGIN).glow.level
        };
        let mut now = 0.0;
        while now < 2.0 {
            level(&state, now);
            now += 1.0 / 144.0;
        }
        state.runtime.tracker.handle_event(NoteEvent::off(now, SourceId::DIRECT, 0, 60));
        let struck = now + 1.0;
        while now < struck {
            level(&state, now);
            now += 1.0 / 144.0;
        }
        let lingering = level(&state, now);
        assert!(lingering > 0.3 && lingering < 0.9, "the halo must be lingering: {lingering}");
        state.runtime.tracker.handle_event(NoteEvent::on(now, SourceId::DIRECT, 0, 72, 1.0));
        let from = now;
        while now < from + 0.3 {
            now += 1.0 / 144.0;
            let got = level(&state, now);
            let floor = lingering * (-(now - from) / RELEASE).exp() as f32 - 1e-4;
            assert!(got >= floor, "at {:.3}s the halo dipped to {got} under {floor}", now - from);
        }
    }

    /// In an octave handoff on one node — one octave let go as another is
    /// struck — the arriving octave's light rises on the Glow attack while the
    /// leaving one's falls on the release (#1784 G2, B′).
    #[test]
    fn an_octave_handoff_brings_one_in_on_the_attack_and_lets_one_go_on_the_release() {
        const ATTACK: f64 = 0.1;
        const RELEASE: f64 = 2.0;
        let mut state = lit(ATTACK as f32, RELEASE as f32);
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let mut fade = GlowFade::default();
        let mut step = |state: &PictureState, now: f64| {
            let mut scene = scene_at(state, now);
            fade.step_drawn(&mut scene, now);
            let node = node_at(&scene, LatticePos::ORIGIN);
            (node.glow, node.glow_light())
        };
        for k in 0..=40 {
            step(&state, f64::from(k) * 0.05);
        }
        let (held, before) = step(&state, 2.0);
        state.runtime.tracker.handle_event(NoteEvent::off(2.0, SourceId::DIRECT, 0, 60));
        state.runtime.tracker.handle_event(NoteEvent::on(2.0, SourceId::DIRECT, 0, 72, 1.0));
        let (_, goal) = step(&state, 2.0);
        let slot = |light: &GlowLight, lit: bool| {
            (0..harmonigraph_scene::OCTAVE_SLOTS).find(|&s| (light.l[s] > 0.0) == lit)
        };
        let leaving = slot(&before, true).expect("C4 lights a slot");
        let arriving = (0..harmonigraph_scene::OCTAVE_SLOTS)
            .find(|&s| goal.l[s] > 0.0 && before.l[s] <= 0.0)
            .expect("C5 lights a slot of its own");
        assert_eq!(goal.l[leaving], 0.0, "C4 must be gone from the reading");
        let mut now = 2.0;
        while now < 2.0 + 3.0 * ATTACK {
            now += 1.0 / 144.0;
            step(&state, now);
        }
        let (glow, _) = step(&state, now);
        let elapsed = now - 2.0;
        let came = glow.light.l[arriving] / goal.l[arriving];
        let kept = glow.light.l[leaving] / held.light.l[leaving];
        let release_keeps = (-elapsed / RELEASE).exp() as f32;
        assert!(came > 0.9, "the arriving octave is only {came:.2} of the way in");
        assert!(
            (kept - release_keeps).abs() < 0.01,
            "the leaving octave kept {kept:.3} where the release keeps {release_keeps:.3}",
        );
    }

    /// Under an attack slower than the release, a melody handoff on a held
    /// light keeps the loop awake until its colour has arrived, and each mark
    /// takes its own direction: the one arriving on the middle octave rises on
    /// the attack, the one leaving the top octave falls on the release.
    #[test]
    fn a_handoff_under_a_slow_attack_repaints_until_its_colour_arrives() {
        const ATTACK: f64 = 2.0;
        const RELEASE: f64 = 0.25;
        let mut state = lit(ATTACK as f32, RELEASE as f32);
        for note in [60, 72, 84] {
            state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, note, 1.0));
        }
        let mut fade = GlowFade::default();
        let mut step = |state: &PictureState, now: f64| {
            let mut scene = scene_at(state, now);
            let moving = fade.step_drawn(&mut scene, now);
            let node = node_at(&scene, LatticePos::ORIGIN);
            let lit = fade.nodes[&LatticePos::ORIGIN];
            (
                node.glow,
                node.glow_light(),
                easing(lit.level, &lit.light, &node.glow_light()),
                moving,
            )
        };
        let mut held = None;
        for k in 0..=200 {
            held = Some(step(&state, f64::from(k) * 0.1));
        }
        let (glow, before, _, _) = held.unwrap();
        assert!(glow.level < before.level(), "the fixture's level landed on its target");
        // The top octave goes: the melody hands down to the middle one.
        state.runtime.tracker.handle_event(NoteEvent::off(20.0, SourceId::DIRECT, 0, 84));
        let strength = state.appearance.view.glow_strength;
        let mut to = None;
        for k in 1..=120 {
            let now = 20.0 + f64::from(k) / 60.0;
            let (glow, goal, easing, awake) = step(&state, now);
            to.get_or_insert(goal);
            assert!(
                awake || easing * strength <= VISIBLE_STEP,
                "at {now:.3}s the colour still had {easing} to move and the loop slept",
            );
            let share = |tau: f64| 1.0 - (-(now - 20.0) / tau).exp() as f32;
            let carried = glow.light;
            let mut expected = before;
            expected.step(&goal, share(ATTACK), share(RELEASE));
            for slot in [6, 7] {
                assert!(
                    (carried.m[slot] - expected.m[slot]).abs() < 1e-4,
                    "at {now:.3}s the mark on slot {slot} is at {} where its own direction \
                     takes it to {}",
                    carried.m[slot],
                    expected.m[slot],
                );
            }
        }
        assert!(to.unwrap().m[6] > 0.0, "the handoff never reached the middle octave");
    }

    /// A surface lights at most `MAX_LIGHTS` nodes, and a light whose node has
    /// left the window holds no place against one arriving: every node of a
    /// window that has moved on entirely is lit again.
    #[test]
    fn a_surface_lights_at_most_max_lights_and_counts_only_what_it_sees() {
        let mut state = lit(0.0, 2.5);
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let template = node_at(&scene_at(&state, 0.0), LatticePos::ORIGIN);
        let window = |from: i32| {
            let mut scene = scene_at(&state, 0.0);
            scene.nodes = (0..=MAX_LIGHTS as i32)
                .map(|i| harmonigraph_scene::NodeInstance {
                    lattice_pos: LatticePos::new(from + i, 0, 0),
                    ..template
                })
                .collect();
            scene
        };
        let lit = |scene: &Scene| scene.nodes.iter().filter(|n| n.glow.level > 0.0).count();
        let mut fade = GlowFade::default();
        let mut first = window(0);
        fade.step_drawn(&mut first, 0.0);
        assert_eq!(lit(&first), MAX_LIGHTS, "one more node than a surface lights");
        // Half a window on: newcomers first in scene order, then the lights
        // still carried, which keep theirs, and the newcomers take only the
        // places those leave.
        let mut half = window(-2048);
        fade.step_drawn(&mut half, 0.05);
        assert_eq!(lit(&half), MAX_LIGHTS, "newcomers ahead of carried lights overran the cap");
        assert!(
            half.nodes.iter().filter(|n| n.lattice_pos.threes >= 0).all(|n| n.glow.level > 0.0),
            "a carried light lost its place to a newcomer",
        );
        let mut moved = window(100_000);
        fade.step_drawn(&mut moved, 0.1);
        assert_eq!(lit(&moved), MAX_LIGHTS, "lights that left the window held their places");
    }

    /// Only a light that is falling holds the frame loop: a sounding note's is
    /// still at its target, and so is a mark held with no key down.
    #[test]
    fn a_light_at_its_target_asks_for_no_repaint() {
        let mut state = lit(0.0, 2.0);
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let mut fade = GlowFade::default();
        for now in [0.0, 0.5, 1.0] {
            let mut scene = scene_at(&state, now);
            let moving = fade.step_drawn(&mut scene, now);
            assert!(!moving, "a held light asked to be repainted at {now}");
        }
        state.runtime.tracker.handle_event(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60));
        let mut scene = scene_at(&state, 1.5);
        let moving = fade.step_drawn(&mut scene, 1.5);
        assert!(moving, "a released light is falling");
    }

    /// A light no layer draws is no light at all (#1836): the octave band
    /// draws a slot's light and the mark strip a mark's, and with both dialled
    /// to nothing the halo would be no pixels (`ink_at` in `lattice.wgsl`). So
    /// a held note lights nothing, and the renderer runs no glow pass for it;
    /// and a light carried while a layer drew it ends once none does, asking
    /// for no frame after.
    #[test]
    fn a_light_no_layer_draws_is_no_light() {
        let held = |band_width: f32, mark_thickness: f32| {
            let mut state = lit(0.0, 2.0);
            // A stack with room for the band, no audio ring under it.
            state.appearance.view.ring_inner = 0.3;
            state.appearance.view.ring_gap = 0.02;
            state.appearance.view.spectral_ring_width = 0.0;
            state.appearance.view.band_width = band_width;
            state.appearance.view.mark_thickness = mark_thickness;
            state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
            let mut fade = GlowFade::default();
            let mut scene = scene_at(&state, 0.0);
            fade.step_drawn(&mut scene, 0.0);
            (state, fade, node_at(&scene, LatticePos::ORIGIN).glow.level)
        };
        assert!(held(0.2, 0.0).2 > 0.99, "a light the band draws");
        assert!(held(0.0, 0.1).2 > 0.99, "a light the mark draws");
        let (_, fade, level) = held(0.0, 0.0);
        assert_eq!(level, 0.0, "a light no layer draws");
        assert!(fade.nodes.is_empty(), "a light no layer draws is carried");

        // Lit on the band, then released and the band dialled off mid-fall.
        let (mut state, mut fade, _) = held(0.2, 0.0);
        state.runtime.tracker.handle_event(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60));
        state.appearance.view.band_width = 0.0;
        let mut scene = scene_at(&state, 1.5);
        let moving = fade.step_drawn(&mut scene, 1.5);
        assert_eq!(node_at(&scene, LatticePos::ORIGIN).glow.level, 0.0, "a light left lit");
        assert!(!moving, "a light no layer draws held the loop");
    }

    /// Under a Mark delay with the band at 0, a lone note's halo arrives with
    /// its mark, on the Glow attack, rather than charging up through the delay
    /// on the band's light, which draws nothing, and appearing already lit
    /// (#1836).
    #[test]
    fn a_halo_waiting_on_a_marks_delay_fades_in_with_it() {
        const DELAY: f32 = 0.5;
        let mut state = lit(0.5, 1.0);
        state.appearance.view.band_width = 0.0;
        state.appearance.view.mark_thickness = 0.1;
        state.appearance.view.mark_delay = DELAY;
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        // One motion across the frames, so the mark waits out its Delay rather
        // than being seeded settled on a fresh surface.
        let mut motion = harmonigraph_scene::NodeMotion::default();
        let mut fade = GlowFade::default();
        let mut frame = |now: f64| {
            let mut scene = harmonigraph_scene::derive_scene(
                &state.runtime.tracker,
                &state.runtime.tuning,
                &state.appearance.view,
                &state.appearance.view.reach(),
                state.appearance.camera,
                None,
            );
            motion.step(
                &mut scene,
                &state.runtime.tracker,
                &state.runtime.tuning,
                &harmonigraph_scene::RingFade::default(),
                now,
            );
            fade.step_drawn(&mut scene, now);
            node_at(&scene, LatticePos::ORIGIN)
        };
        let mut now = 0.0;
        let node = loop {
            let node = frame(now);
            if node.melody_level > 0.0 {
                break node;
            }
            assert!(now < 2.0 * f64::from(DELAY), "the mark never appeared");
            now += 1.0 / 144.0;
        };
        assert!(now >= f64::from(DELAY) - 0.01, "the mark did not wait: {now}");
        assert!(node.glow.level < 0.05, "the halo arrived lit: {}", node.glow.level);
        let later = frame(now + 1.0).glow.level;
        assert!(later > 0.5, "the halo never came in: {later}");
    }

    /// The step that ends a light, snapping it to exactly 0, asks for no
    /// repaint of its own: at a gain of 1 the level just under `GONE` is still
    /// over a visible step, and the frame after it would draw nothing new.
    #[test]
    fn the_step_that_ends_a_light_asks_for_no_repaint() {
        let mut state = lit(0.0, 0.25);
        state.appearance.view.glow_strength = 1.0;
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let mut fade = GlowFade::default();
        let mut scene = scene_at(&state, 0.0);
        fade.step_drawn(&mut scene, 0.0);
        assert!(node_at(&scene, LatticePos::ORIGIN).glow.level > 0.0, "the note lit nothing");
        state.runtime.tracker.handle_event(NoteEvent::off(0.0, SourceId::DIRECT, 0, 60));
        let mut now = 0.0;
        loop {
            now += 1.0 / 144.0;
            let mut scene = scene_at(&state, now);
            let moving = fade.step_drawn(&mut scene, now);
            if node_at(&scene, LatticePos::ORIGIN).glow.level == 0.0 {
                assert!(!moving, "the light ended at {now:.3}s and asked for another frame");
                break;
            }
            assert!(now < 10.0, "the light never ended");
        }
    }

    /// Clear display history keeps a held note's halo where it is and drops a
    /// released one's: under an attack, a held light that restarted would
    /// start dark and fade back up.
    #[test]
    fn a_clear_keeps_held_halos_and_drops_released_ones() {
        let mut state = lit(0.5, 2.5);
        for note in [60, 64] {
            state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, note, 1.0));
        }
        // Every lit node's level, and whether its own layers still light it.
        let levels = |state: &mut PictureState, now: f64| {
            let mut scene = scene_at(state, now);
            apply(&mut scene, state, 0, now);
            let lit = scene.nodes.iter().filter(|n| n.glow.level > 0.0);
            lit.map(|n| (n.lattice_pos, n.glow.level, n.glow_target() > 0.0)).collect::<Vec<_>>()
        };
        levels(&mut state, 0.0);
        levels(&mut state, 1.0);
        state.runtime.tracker.handle_event(NoteEvent::off(1.0, SourceId::DIRECT, 0, 64));
        let before = levels(&mut state, 1.5);
        let held: Vec<_> = before.iter().filter(|n| n.2).copied().collect();
        assert!(held.iter().any(|n| n.0 == LatticePos::ORIGIN), "the held C is not lit");
        assert!(held.len() < before.len(), "the released E's halo was already gone");
        state.clear_accumulated();
        // At the same clock, so a carried light moves by nothing and a new one
        // starts dark.
        assert_eq!(levels(&mut state, 1.5), held, "a held halo moved, or a released one stayed");
    }

    /// A new light's level attacks from darkness, and what it is made of
    /// starts as the node's whole reading.
    #[test]
    fn a_new_glow_attacks_from_darkness() {
        let mut state = lit(1.0, 0.5);
        let mut fade = GlowFade::default();
        let mut idle = scene_at(&state, 0.0);
        fade.step_drawn(&mut idle, 0.0);
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        for (time, expected) in
            [(0.0, 0.0), (0.5, 1.0 - (-0.5f32).exp()), (1.0, 1.0 - (-1.0f32).exp())]
        {
            let mut scene = scene_at(&state, time);
            fade.step_drawn(&mut scene, time);
            let node = node_at(&scene, LatticePos::ORIGIN);
            assert!((node.glow.level - expected).abs() < 0.001, "attack at {time}: {node:?}");
            assert_eq!(node.glow.light, node.glow_light(), "the light was not seeded whole");
        }
    }

    /// The attack and release over a clock that ran BACKWARDS hold the light
    /// where it is, as a frame drawn twice at one time does.
    #[test]
    fn a_glow_clock_that_runs_backwards_holds_the_light() {
        assert_eq!(coefficients(1.0, Some(2.0), 0.05, 0.5), (0.0, 0.0));
        assert_eq!(coefficients(1.0, Some(1.0), 0.05, 0.5), (0.0, 0.0));
        let (up, down) = coefficients(1.0, Some(0.9), 0.05, 0.5);
        assert!(up > down && down > 0.0 && up < 1.0, "{up} and {down} over a tenth of a second");
    }

    /// An audio ring holds NO light, however loud what it is reading is.
    ///
    /// A halo says something is being played at this position, and the ring
    /// says something is being heard in the room — the node's own voice against
    /// a reading it wears. So the ring is drawn and never shone: neither the
    /// level nor anything the light is made of reads it.
    ///
    /// Measured with the analyzer at BOTH its ends at once, which is what makes
    /// the claim about the layer rather than about one of its numbers: a whole
    /// ring showing (the Gate at its floor admits every node in the window) and
    /// a grid full-scale everywhere in it. Held with no key anywhere, so what a
    /// light would be reading is the ring alone.
    #[test]
    fn an_audio_ring_holds_no_light() {
        let state = lit(0.0, 2.5);
        let mut scene = scene_at(&state, 0.0);
        // A dialled-in annulus, which is what `ring_draws` asks: without one the
        // ring layer is off and there is nothing for the light to leave out.
        scene.spectral.inner = 0.2;
        scene.spectral.outer = 0.4;
        scene.spectral.levels.fill(u8::MAX);
        let node = scene.nodes.first_mut().expect("the window holds a node");
        assert_eq!(node.presence_with_opacity, 0.0, "the fixture must hold no key");
        node.audio_ring = 1.0;
        let pos = node.lattice_pos;

        let mut fade = GlowFade::default();
        fade.step_drawn(&mut scene, 0.0);
        let held = node_at(&scene, pos).glow.level;
        assert_eq!(held, 0.0, "a ring lit its node at {held}");
        assert!(fade.nodes.is_empty(), "a ring was given a light");
    }

    /// The light off is the light not stepped: no state, and every node's step
    /// left exactly as `derive_scene` handed it over.
    #[test]
    fn a_view_with_no_light_carries_nothing() {
        let mut state = lit(0.3, 2.5);
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let mut scene = scene_at(&state, 0.0);
        let before = node_at(&scene, LatticePos::ORIGIN).glow;
        state.appearance.view.glow_reach = 0.0;
        scene.view.glow_reach = 0.0;
        apply(&mut scene, &mut state, 0, 0.0);
        assert_eq!(node_at(&scene, LatticePos::ORIGIN).glow, before);
        assert!(state.surfaces.glow_fade.is_empty(), "a light that is off kept state");
    }
}
