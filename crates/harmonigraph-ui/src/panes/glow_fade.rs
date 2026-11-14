//! The node glow's own clock: how a node's light follows the node.
//!
//! Every drawn layer of a node rides the note Fade — the octave band, the
//! marks, the audio ring — and the light is deliberately not one of them.
//! A halo is the slow part of the picture: stepped with the marks' own fast
//! envelopes it flickers with them, and what a light should do instead is
//! arrive a little behind the note and leave a long way behind it. So it has a
//! pair of its own ([`ViewConfig::glow_attack`] and `_release`) and this is
//! where they are spent.
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
//! node's [`LatticePos`], and stepped with ONE coefficient per frame, so the two
//! cannot come to disagree about which way a light is going (#1626).
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
use harmonigraph_scene::{GlowLight, GlowStep, Scene, ViewConfig, MAX_LIGHTS};

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
/// under it has nothing left to repaint for ([`GlowFade::fading_at`]).
///
/// Measured as level times the Background glow gain, which is the shader's
/// peak without its `GLOW_BASE` of 0.8, so the window errs toward staying
/// awake a little long rather than stopping on a visible step.
const VISIBLE_STEP: f32 = 0.5 / 255.0;

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
    /// When the largest falling light stops moving a pixel, as of the last
    /// step: the clock at which its distance above its target times the glow
    /// gain falls under [`VISIBLE_STEP`], at the Glow release the filter falls
    /// at. `None` when no light is falling. What the frame loop stays awake
    /// for (`releasing` in `lib.rs`); a surface that stops being drawn stops
    /// moving it, and it lapses on its own.
    visible_until: Option<f64>,
}

/// One node's light: where its level and what it is made of have got to.
#[derive(Clone, Copy)]
struct Lit {
    level: f32,
    light: GlowLight,
    /// The node's [`glow_target`](harmonigraph_scene::NodeInstance::glow_target)
    /// as of this step, for the two passes after the walk over the nodes (which
    /// lights to keep, and how long the falling ones stay visible). Written on
    /// every step that sees the node, and a light not seen is dropped, so it
    /// never outlives the step it was read on.
    target: f32,
    /// The step this node was last seen on (see [`GlowFade::frame`]).
    seen: u64,
}

/// Carry every node's light toward what its layers say now.
///
/// Runs behind [`NodeMotion::step`](harmonigraph_scene::NodeMotion::step), which is
/// what a filter needs: the light it steps toward is read off the node's
/// layers, and there is nothing to step until the frame has written them. Its
/// place in the order of the passes is that and no more — the fold ahead of it
/// settles the audio ring, which is not a light (see [`GlowFade::step`]'s
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
    glow.step(scene, &state.appearance.view, now);
    state.surfaces.moving_this_frame.releasing(glow.fading_at(now));
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
    fn step(&mut self, scene: &mut Scene, view: &ViewConfig, now: f64) {
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
        // Two coefficients for the whole frame: every node's light runs on the
        // same pair of times, and which of the two it takes is which way its
        // level is going.
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
            let goal = node.glow_light();
            let target = goal.level();
            let full = self.nodes.len() >= MAX_LIGHTS;
            node.glow = match self.nodes.get_mut(&node.lattice_pos) {
                Some(lit) => {
                    lit.seen = self.frame;
                    lit.target = target;
                    let mix = if target > lit.level { up } else { down };
                    lit.level += (target - lit.level) * mix;
                    lit.light.step(&goal, mix);
                    // The end of a light, said exactly rather than approached:
                    // a level that only gets small keeps an entry and an
                    // instance for the rest of the session. Only where there
                    // is nothing left to follow — a target that is small but
                    // real is a node still drawing something.
                    if lit.level < GONE && target <= 0.0 {
                        lit.level = 0.0;
                    }
                    GlowStep { level: lit.level, light: Some(lit.light) }
                }
                // A node with no light yet and none arriving is left alone, and
                // so is one past the most a surface lights at once — its
                // layers draw exactly as they do with the glow off.
                None if target <= 0.0 || full => GlowStep::default(),
                None => {
                    let level = target * seed;
                    self.nodes.insert(
                        node.lattice_pos,
                        Lit { level, light: goal, target, seen: self.frame },
                    );
                    GlowStep { level, light: Some(goal) }
                }
            };
        }
        // Every node whose light this frame did not see has left the window, so
        // there is nothing to carry and nothing to draw.
        let frame = self.frame;
        self.nodes.retain(|_, lit| lit.seen == frame && (lit.level > 0.0 || lit.target > 0.0));
        // Only a light above its target is moving on its own: one that is
        // rising belongs to a note being played, and one at its target is
        // still, so a mark held with no key down never keeps the loop awake.
        // A level falls as `exp(-t / release)` toward its target, exactly, at
        // whatever cadence the frames come.
        let falling = self.nodes.values().map(|lit| lit.level - lit.target).fold(0.0, f32::max);
        let seen = falling * scene.view.glow_strength;
        self.visible_until = (seen > VISIBLE_STEP)
            .then(|| now + f64::from(view.glow_release) * f64::from(seen / VISIBLE_STEP).ln());
    }

    /// Whether some node's light is still falling by a visible step at `now`.
    pub(crate) fn fading_at(&self, now: f64) -> bool {
        self.visible_until.is_some_and(|until| until > now)
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
            &state.runtime.frame_params,
            state.appearance.camera,
            None,
        );
        harmonigraph_scene::NodeMotion::default().step(
            &mut scene,
            &state.runtime.tracker,
            &state.runtime.tuning,
            &state.appearance.view,
            &state.appearance.view.envelope(&state.runtime.frame_params),
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
    /// Fade at nothing so what a layer does is a step and what the LIGHT does
    /// is the only thing being filtered.
    fn lit(attack: f32, release: f32) -> PictureState {
        let mut state = fresh();
        state.appearance.view.glow_reach = 0.8;
        state.appearance.view.glow_attack = attack;
        state.appearance.view.glow_release = release;
        state.runtime.frame_params.fade_time = 0.0;
        state
    }

    /// Velocity to Opacity over a rest of about a third, as the routes stood
    /// fresh until #1475, and pressure to Thickness: so a soft note's slots
    /// light short of full and a press swells a held slice.
    fn routed() -> harmonigraph_scene::IntensitySettings {
        use harmonigraph_scene::IntensitySource;
        harmonigraph_scene::IntensitySettings {
            velocity: IntensitySource { opacity: Some(0.6), thickness: None },
            pressure: IntensitySource { opacity: None, thickness: Some(1.0) },
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
        fade.step(&mut scene, &state.appearance.view, 0.0);
        let held = node_at(&scene, LatticePos::ORIGIN).glow.level;
        assert!(held > 0.99, "a held note lights its node at once: {held}");

        // The key comes up, and with no Fade under it the node draws nothing
        // at all from the next frame on.
        state.runtime.tracker.handle_event(NoteEvent::off(0.0, SourceId::DIRECT, 0, 60));
        let level = |fade: &mut GlowFade, state: &PictureState, now: f64| {
            let mut scene = scene_at(state, now);
            fade.step(&mut scene, &state.appearance.view, now);
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
            fade.step(&mut scene, &state.appearance.view, now);
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
        let light = |step: &GlowStep| step.light.expect("a lit node carries its light").l[slot];
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
    ///   full (#1315);
    /// - pressure routed to Thickness, pressed on a held note: the slice's
    ///   reach, with its light unmoved.
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
        let pressure = NoteEvent {
            time: AT,
            source: SourceId::DIRECT,
            channel: 0,
            note: 60,
            kind: harmonigraph_core::NoteEventKind::Expression {
                expression: harmonigraph_core::Expression::Pressure,
                value: 1.0,
            },
        };
        type Reach = fn(&GlowLight, &GlowLight) -> bool;
        let rows: [(&str, Vec<NoteEvent>, Vec<NoteEvent>, Reach); 4] = [
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
            ("pressure to thickness", vec![on(60, 1.0)], vec![pressure], |a, b| {
                a.l == b.l && a.p != b.p
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
                fade.step(&mut scene, &state.appearance.view, now);
                let node = node_at(&scene, LatticePos::ORIGIN);
                (node.glow.light.expect("the node is lit"), node.glow_light())
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
                expected.step(&to, share);
                let fields = |light: &GlowLight| [light.l, light.p, light.m].concat();
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

    /// Only a light that is falling holds the frame loop: a sounding note's is
    /// still at its target, and so is a mark held with no key down.
    #[test]
    fn a_light_at_its_target_asks_for_no_repaint() {
        let mut state = lit(0.0, 2.0);
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let mut fade = GlowFade::default();
        for now in [0.0, 0.5, 1.0] {
            let mut scene = scene_at(&state, now);
            fade.step(&mut scene, &state.appearance.view, now);
            assert!(!fade.fading_at(now), "a held light asked to be repainted at {now}");
        }
        state.runtime.tracker.handle_event(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60));
        let mut scene = scene_at(&state, 1.5);
        fade.step(&mut scene, &state.appearance.view, 1.5);
        assert!(fade.fading_at(1.5), "a released light is falling");
    }

    /// A new light's level attacks from darkness, and what it is made of
    /// starts as the node's whole reading.
    #[test]
    fn a_new_glow_attacks_from_darkness() {
        let mut state = lit(1.0, 0.5);
        let mut fade = GlowFade::default();
        let mut idle = scene_at(&state, 0.0);
        fade.step(&mut idle, &state.appearance.view, 0.0);
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        for (time, expected) in
            [(0.0, 0.0), (0.5, 1.0 - (-0.5f32).exp()), (1.0, 1.0 - (-1.0f32).exp())]
        {
            let mut scene = scene_at(&state, time);
            fade.step(&mut scene, &state.appearance.view, time);
            let node = node_at(&scene, LatticePos::ORIGIN);
            assert!((node.glow.level - expected).abs() < 0.001, "attack at {time}: {node:?}");
            assert_eq!(node.glow.light, Some(node.glow_light()), "the light was not seeded whole");
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
        fade.step(&mut scene, &state.appearance.view, 0.0);
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

    /// Every pass egui lays out at one clock steps the light by nothing, so a
    /// pass it discards spends none of the attack.
    #[test]
    fn discarded_layout_passes_do_not_skip_the_attack() {
        let mut state = lit(0.3, 2.5);
        state.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        let ctx = egui::Context::default();
        let mut steps = Vec::new();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::Grid::new("glow-multipass").show(ui, |ui| {
                ui.label("A new grid requests its sizing pass automatically");
            });
            let mut scene = scene_at(&state, 0.0);
            apply(&mut scene, &mut state, 0, 0.0);
            steps.push(node_at(&scene, LatticePos::ORIGIN).glow);
            assert_eq!(scene.glow_clock, Some(0.0));
        });
        assert!(steps.len() >= 2, "the fixture must discard an actual layout pass");
        assert!(
            steps.iter().all(|step| step.level == 0.0 && step.light.is_some()),
            "discarded passes skipped the attack: {steps:?}",
        );
    }
}
