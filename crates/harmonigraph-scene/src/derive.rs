//! Per-frame scene derivation: turns the note tracker + tuning into the
//! node/edge instance lists the renderer draws. Animation and envelope
//! *policy* lives here.

use crate::camera::Camera;
use crate::color::{pitch_lut_color, pitch_ramp_lut};
use crate::octaves::octave_layout;
use crate::trail::TrailField;
use crate::view::{DrawnWindow, FrameParams, ViewConfig};
use crate::{
    lattice_to_world, EdgeInstance, GlowStep, NodeInstance, Pulse, Scene, SpectralPaint,
    MARK_DELAY_MAX, NODE_RADIUS_FACTOR, OCTAVE_SLOTS,
};
use glam::Vec4;
use harmonigraph_core::{HeldEnd, LatticePos, NoteTracker, Time, Tuning, VoiceState};

/// A melody or bass mark accumulator for one node: the octave slot it marks,
/// plus the color and drawn level, all three read off the STRONGEST marking
/// voice — the voice's envelope times how far its mark has eased in. The `>=`
/// in [`Mark::add`] means ties favor the later voice, so a release
/// crossfading two voices lands on the newer color.
///
/// One voice entire, rather than the union of every marking voice's slot
/// under the strongest one's level, because the node carries ONE level: a
/// weaker voice's slot admitted alongside it would be drawn at the winner's
/// brightness. That is not a near-miss — it is a mark on an octave that is
/// half gone (or has not started), at full. Two voices reach one node's mark
/// only through a handoff inside a single pitch class, where the loser is by
/// definition the dimmer of a crossfading pair, so what the union would buy
/// is exactly the reading that cannot be drawn honestly.
///
/// The cost is that the mark JUMPS to the other sector at the crossover
/// instead of both being lit through it, and the jump is the whole SHAPE
/// moving one wedge round the node — a mark is its sector's slice continued
/// outward, so it has no body that stays put while the sector changes under
/// it. The level is continuous across the switch (the two curves are equal at
/// the moment the argmax changes), so what moves is the position and not the
/// brightness. Lighting both honestly wants a level per slot, which is
/// `OCTAVE_SLOTS` more floats per node in the instance buffer and a shader
/// that reads them; that is the price, and it buys one frame's worth of a
/// second mark in one voicing.
#[derive(Default)]
struct Mark {
    /// The slot as a MASK, so 0 is "unmarked" without an `Option` — which is
    /// also how the shader reads it (see `NodeInstance::melody_slots`). One
    /// bit at a time, per the argmax above.
    slots: u32,
    level: f32,
    color: Vec4,
}

impl Mark {
    fn add(&mut self, slot: usize, level: f32, color: Vec4) {
        if level >= self.level {
            self.slots = 1 << slot;
            self.level = level;
            self.color = color;
        }
    }
}

/// One voice's per-frame envelope work: everything about how brightly it
/// draws that depends on the voice, the frame and `now` alone, done once
/// rather than on every node the voice matches.
struct FrameVoice<'a> {
    voice: &'a harmonigraph_core::Voice,
    /// The pitch color the DISC and its octave sector take. The melody/bass
    /// marks do not reuse it — they belong to the octave layer, colored by
    /// axis position (see the mark color in [`derive_scene`]).
    color: Vec4,
    /// The note's own envelope, attack times what is left of the release:
    /// what the disc, the glow, the gutter and the octave sector all draw at.
    activation: f32,
    /// Whether this voice's departure has begun (see
    /// [`NodeInstance::departing`](crate::NodeInstance::departing)).
    departing: bool,
    /// What this voice's melody mark draws at, or `None` where it wears no
    /// melody end. The RELEASE alone under the mark's own ease — see the ease
    /// in [`derive_scene`], and
    /// [`Voice::activation`](harmonigraph_core::Voice::activation) for why the
    /// note's attack is not multiplied in on top.
    melody: Option<f32>,
    /// The same for the bass end.
    bass: Option<f32>,
}

/// The highest and lowest HELD voices with the moment each took its end, as
/// the caller asked for them — either is `None` when that end isn't being
/// marked or nothing is held.
///
/// Held only, which is the tracker's own answer ([`HeldEnd`]): these are the
/// LIVE ends, the notes actually down, and a released voice is never one of
/// them however brightly its mark is still drawn. What a released voice wears
/// is the stamp it left with
/// ([`Voice::wore_high`](harmonigraph_core::Voice::wore_high)), read in
/// [`marks`] — a mark on its way out, not a claim on the end.
///
/// The two must not be conflated: a released voice allowed back in here would
/// keep the end from the note that replaced it, and the incoming mark would
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
/// the held set with, so its mark fades out with the note instead of snapping
/// off at the key — which is the same envelope every other layer of the node
/// leaves on, and the reason a handoff reads as one mark crossing to another
/// rather than as one vanishing and a second appearing.
///
/// The released stamps are gated on the same two flags here that
/// [`held_extremes`] applies to the live ends, and on the flags as they are
/// NOW. They have to be: a stamp is left whatever the view says, so a mark
/// turned off mid-fade would otherwise go on drawing until the note it
/// belongs to is pruned.
///
/// Turned back on mid-fade, the marks of notes released while it was off
/// appear at the level their fade has reached. That is a toggle behaving like
/// a toggle rather than a hole — switching the melody on over a held chord
/// puts its mark up at full the same frame, the end having been taken long
/// ago. The alternative is recording the flag at the release, which is a view
/// setting baked into the tracker: the stamp is a fact about the music, and
/// what is drawn from it is the view's to re-answer every frame (see
/// [`Voice::wore_high`](harmonigraph_core::Voice::wore_high)).
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
///
/// `window` is which nodes to build and `view` is how they look — the split
/// that lets two panes draw one view at two aspects. It is the pane's own
/// ([`ViewConfig::scrolled`]); handing it the view's naming
/// [`reach`](ViewConfig::reach) instead draws a picture that is subtly the
/// wrong size, which is why the two are different types.
#[allow(clippy::too_many_arguments)]
pub fn derive_scene(
    tracker: &NoteTracker,
    tuning: &Tuning,
    view: &ViewConfig,
    window: &DrawnWindow,
    frame: &FrameParams,
    camera: Camera,
    hovered: Option<LatticePos>,
    now: f64,
) -> Scene {
    let mut nodes = Vec::with_capacity(window.count());
    // Kept parallel to `nodes` for the trail, which matches remembered
    // pitches against every node afterwards and would otherwise have to
    // recompute each node's pitch class to do it.
    let mut node_pcs = Vec::with_capacity(window.count());
    let center = view.center();
    // The lattice AT REST, resolved once for the frame: what the grid lines
    // draw in, what both of a node's rings stand on where nothing is lit, and
    // the neutral an unplayed node falls back to. One resolve rather than
    // three, so the three cannot answer differently.
    let ground = crate::grey_of_lightness(view.lattice_ground_lightness());
    // Read by nothing that draws while a node stays unplayed -- an idle node
    // paints no pixel -- so this is a fallback rather than a look. The ground
    // rather than an arbitrary grey so that a node arriving or leaving crosses
    // no seam against the grid lines running into it.
    let node_idle = ground;
    let live_extremes = held_extremes(tracker, view.mark_melody, view.mark_bass);
    let mark_delay = view.mark_delay.clamp(0.0, MARK_DELAY_MAX) as f64;
    let env = view.envelope(frame);
    // How far a mark taken at `since` has eased in, for the voice `state`.
    //
    // The delay simply moves the ramp's start later, so an end held for less
    // than it never draws a mark at all. That is what the setting is for: at
    // speed the ends change hands every few frames, and a mark easing in on
    // each of them reads as flicker rather than as the top line.
    //
    // A mark outlives its key, so for a RELEASED voice the wait is checked as
    // a threshold at the key-up and the ramp then runs on at `now` like every
    // other layer's. The two are separate rules and the threshold is the one
    // the delay is: a note that gave the end back inside its wait never rang
    // while it was down, and an ease left running would sail past the
    // threshold during the release and put a mark on it afterwards — the very
    // flicker the setting buys off.
    //
    // The RAMP is not that rule, and freezing it where the key happened to
    // find it costs the other half: a note shorter than the attack would keep
    // a mark dimmer than the sector it extends for its whole release, since
    // the disc's own attack keeps climbing past the note-off (see
    // `Envelope::attack`). One layer arriving slower than the next is exactly
    // the disagreement one shared curve exists to prevent.
    let ease = |since: Time, state: VoiceState| {
        if let VoiceState::Released { at } = state {
            if at < since + mark_delay {
                // Never earned a mark, so there is none to fade out.
                return 0.0;
            }
        }
        env.attack(now, since + mark_delay)
    };
    // Sanitized once, outside the node loop. Capped at 1: this axis makes
    // off-sheet nodes SMALLER, never larger, so the home sheet stays the
    // biggest thing on screen (see `ViewConfig::sevens_size`). The floor
    // keeps a sheet from collapsing to an invisible speck at extent 4.
    let sevens_size = view.sevens_size.clamp(0.15, 1.0);
    // Bounded well inside the billboard: the quad reaches QUAD_MARGIN (1.6)
    // in uv, and the gutter has to finish inside it or it would be clipped
    // square instead of ending on the node's own outline.
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

    // Each voice's color and envelope, computed once here rather than re-run
    // on every node the voice matches. All of it depends on the voice, the
    // frame and `now` alone — never on the node — so this lifts the ramp walk
    // and the envelope's four `powf`s (two ends, times the disc and the mark)
    // out of the O(nodes × voices) loop below. One pitch class lights every
    // node that spells it, so on a wide window that is tens of nodes per
    // voice.
    //
    // What genuinely varies per node stays down there: which octave slot the
    // voice sounds in on that node, and the mark color read off that slot's
    // own pitch.
    let voices: Vec<FrameVoice> = tracker
        .voices()
        .map(|voice| {
            // The voice's OWN pitch. Nothing here asks which channel carried
            // it: a channel is a routing detail of the host's, and two notes
            // of one pitch draw identically whichever lanes they arrived on.
            let color = pitch_lut_color(
                voice.pitch,
                frame.darkest_pitch,
                frame.brightest_pitch,
                view.pitch_gradient,
            );
            // Which ends this voice wears is per voice too — the live ends are
            // a frame-wide answer and the stamps are the voice's own.
            let (melody_since, bass_since) =
                marks(voice, live_extremes, view.mark_melody, view.mark_bass);
            // The RELEASE alone under the mark's own ease, not the node's full
            // activation: the attack is in that, and the mark already carries
            // one from the moment its note took the end. Multiplying both in
            // would square the ramp wherever those two moments coincide —
            // which is the ordinary case, a note arriving as the new outer
            // voice — and a mark rising as the square of the sector it
            // extends is precisely the disagreement about how fast the note
            // arrived that one shared rate exists to prevent.
            //
            // The release this rides waits out the NOTE's arrival
            // (`Voice::release_level`), which is the rule that keeps a stab's
            // DISC at full — and it covers the mark only where the mark's own
            // ease starts with the note. Whatever moves that ease later moves
            // it out from under the rule, and both things that can are
            // deliberate: the mark Delay, and an end taken by inheritance
            // part way through a note.
            //
            // So a mark comes in GRADED over the band between the Delay and
            // the Delay plus the Fade. At the fresh view (Delay 0.15, Fade
            // 0.15) a note held 160ms peaks at 0.30, 200ms at 0.57, 250ms at
            // 0.89, and 300ms and longer at full — while the disc under it is
            // at full for every one of them. That band is the threshold
            // softened rather than a second one: the Delay's own claim is
            // that an end held briefly should not read as the line being
            // traced, and a Delay of 0 puts the mark back exactly on the
            // note's rule.
            let release = voice.release_level(now, &env);
            let mark = |since: Option<Time>| since.map(|s| release * ease(s, voice.state));
            FrameVoice {
                voice,
                color,
                activation: voice.activation(now, &env),
                // Below full is the departure under way, and only that: the
                // release holds at 1 until the key comes up AND the arrival
                // has landed, so this cannot be a note still easing in.
                departing: release < 1.0,
                melody: mark(melody_since),
                bass: mark(bass_since),
            }
        })
        .collect();

    for pos in window.positions() {
        let node_pc = tuning.pitch_class(pos);
        let node_cents = node_pc.to_cents();
        // The octaves of THIS pitch class nearest the center pitch, which is
        // what its indicators are. Per node rather than per frame: where a
        // class falls against the center decides which octaves of it are the
        // nearest ones, and the ring is turned to match.
        let (low_slot, high_slot) = octave_layout.slots(node_cents);
        let mut activation = 0.0f32;
        // Follows the voice that WINS the activation, so the flag describes
        // the same voice the node is lit and colored by rather than any other
        // one that happens to match this pitch class.
        let mut departing = false;
        let mut octaves = [0f32; OCTAVE_SLOTS];
        let mut color = node_idle;
        let mut melody = Mark::default();
        let mut bass = Mark::default();

        // O(nodes × voices); fine at this scale. If extents grow large,
        // index voices by quantized pitch class instead.
        for lit in &voices {
            let voice = lit.voice;
            if tuning.matches(voice.pitch_class, node_pc) {
                let envelope = lit.activation;
                if envelope > activation {
                    activation = envelope;
                    color = lit.color;
                    departing = lit.departing;
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
                // The mark is the strongest marking voice ON THIS NODE,
                // entire (see [`Mark`]). It eases in while its end is held
                // and fades out with the note when the key comes up, on the
                // note's own release — so a handoff is one mark crossing to
                // another rather than one vanishing as a second appears.
                if lit.melody.is_some() || lit.bass.is_some() {
                    // The mark takes the color of the SECTOR it links back to
                    // — the pitch of that slot on this node, through the very
                    // table the shader tints the lit glyph from — so the mark
                    // is never a shade off the one indicator it is pointing
                    // at. The voice's own color is the wrong one to reuse
                    // here: a note past either end of the ring folds onto
                    // the outermost slot, so its mark would carry a pitch
                    // that is nowhere on the axis it is drawn around.
                    //
                    // Both pitches are on the one ramp, so which pitch is read
                    // is the whole of the difference: the shader tints a lit
                    // glyph by the pitch it is DRAWN at and asks nothing about
                    // the voice, and a mark is part of that glyph's layer.
                    // Only the lit glyph — the band's unsounding slices wear
                    // the rings' own ground and a solo voice's glow keeps the
                    // voice's color, both of them on purpose. No extra lift on top of
                    // the ramp here: the sector's glyph wears it as it comes,
                    // and a lightened mark would read a shade off the slice
                    // it continues.
                    //
                    // The mark eases in on the SAME ramp as that sector, from
                    // when the note took the end — which is not always its
                    // note-on, and is the tracker's own answer rather than
                    // anything derived here (`HeldEnd` while the note is
                    // down, `Voice::wore_high` once it is not). That level is
                    // the voice's own and is computed with it; the color is
                    // what has to be read here, off this node's slot.
                    let mark_color = pitch_lut_color(
                        octave_layout.slot_pitch(slot as i32, node_cents),
                        frame.darkest_pitch,
                        frame.brightest_pitch,
                        view.pitch_gradient,
                    );
                    if let Some(level) = lit.melody {
                        melody.add(slot, level, mark_color);
                    }
                    if let Some(level) = lit.bass {
                        bass.add(slot, level, mark_color);
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
        // EVERY node that DRAWS clears what is behind it, the home sheet
        // included. Leaving the home sheet out is what let the sheets
        // behind it show straight through the gaps in a home node's body —
        // a node is annuli with gaps between them, so "drawn over" covers
        // very little — and neither sheet then read as being in front of the
        // other.
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
        // The WIDTH is a constant of the view; the STRENGTH is per LAYER, and
        // the shader is where it is applied — each layer's hole scaled by the
        // same level that paints that layer, so a clearing fades out exactly as
        // the ink in it does. Scaling the width by an envelope instead leaves
        // the clearing fully opaque for the whole release and only narrows its
        // soft edge, so the hole hangs around at full strength and then
        // vanishes the instant the voice is pruned.
        //
        // Which is why this is not gated on `activation` here, though it reads
        // like it should be: a node the keys never touched still draws its audio
        // ring wherever the view's Gate lets it, and a node that draws ink owes
        // that ink a hole. The gate for THIS field would have to be "does any
        // layer draw", and the ring's half of that answer is not known yet —
        // `Scene::wear_audio_rings` fills it in after the fold. So the width is
        // handed over whole and the shader, which has every level, decides what
        // clears. A node with no layer drawing at all clears nothing there, and
        // is culled before it reaches the shader anyway.
        let gutter = sevens_gutter;

        nodes.push(NodeInstance {
            lattice_pos: pos,
            world_pos,
            color,
            activation,
            departing,
            octaves,
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
            // Nothing has been measured yet, so nothing can be held back: the
            // audio channel arrives empty here and `Scene::wear_audio_rings` is
            // what answers this once the shell's fold has filled it.
            audio_ring: 1.0,
            // The light UNCARRIED: what the MIDI layers say right now, on this
            // node's own row of a strip one row per node tall, taken whole.
            // The audio ring is left out of the level for the same reason it
            // arrives at 1 above — nothing here has measured it — and the
            // shell's `panes::glow_fade` is what settles both.
            glow: GlowStep {
                level: activation.max(melody.level).max(bass.level),
                row: nodes.len() as u32,
                mix: 1.0,
                marked: f32::from((melody.slots | bass.slots) != 0),
            },
            trail: 0.0,
        });
        node_pcs.push(node_pc);
    }

    // Writes `trail` and nothing else (see `trail`), so nothing downstream
    // that reads "is sounding" — the grid's sevens chains above all — can
    // pick a memory up by mistake, whatever order these run in. The label
    // layer is the only reader, and it draws no shape.
    if let Some(field) = TrailField::build(tracker.history(), view) {
        field.apply(&mut nodes, &node_pcs, tuning);
    }

    let nodes_len = nodes.len() as u32;
    let grid = derive_grid(view, window, &nodes, ground);

    // Every radius on a node, off the one stack the size bars describe
    // (`ViewConfig::rings`, which is also where their clamps live): each ring
    // is a width a gap out from whatever is inside it — or from the node's own
    // center, for the innermost one on — and a layer dialled to 0 is off and
    // hands its slot back. The shader can trust outer > inner on a band that
    // draws at all, and an empty pair is the one thing that says a ring does
    // not.
    let rings = view.rings();

    Scene {
        nodes,
        camera,
        now,
        node_radius: view.spacing * NODE_RADIUS_FACTOR,
        outer_inner: rings.band.0,
        outer_outer: rings.band.1,
        rings_outer: rings.outer,
        mark_inner: rings.mark_inner,
        octave_gap: view.octave_gap_width(),
        lattice_ground: ground,
        // The MIDI picture, whole: nothing here reads audio, so the audio
        // channel arrives empty and the Lattice pane's fold is what fills it.
        spectral: SpectralPaint::silent(),
        octave_layout,
        grid,
        grid_thickness: view.grid_thickness.clamp(0.0, 8.0),
        mark_thickness: rings.mark_thickness,
        // A mark sheet reaches the extension AND the octave slice it extends,
        // so with the mark layer off — no end marked, or no depth to
        // draw one with, which is `marks_draw` and is exactly what the
        // pane grays the row on — the extension half is multiplied away by a
        // coverage of zero and the SLICE half is not: the glyph layer draws
        // that either way, so without this fold the sheet would go on
        // sweeping from a control the user can no longer reach. Folded here
        // so the picture and the control agree by construction.
        pulse_marks: if view.marks_draw() { view.pulse_marks } else { Pulse::Off },
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
        background: crate::skin::well_color(),
        pitch_lut: pitch_ramp_lut(view.pitch_gradient),
        darkest_pitch: frame.darkest_pitch,
        brightest_pitch: frame.brightest_pitch,
        render_scale: view.render_scale,
        bloom_strength: view.bloom_strength,
        // Clamped here as well as in `sanitize`, for the shells that never come
        // through that door: the reach and the gap both size the billboard the
        // glow's draws use, so a number from outside either bar is a quad they
        // cannot fill.
        glow_reach: view.glow_reach.clamp(0.0, crate::GLOW_REACH_MAX),
        glow_strength: view.glow_strength.clamp(0.0, crate::GLOW_STRENGTH_MAX),
        glow_feather: view.glow_feather.clamp(0.0, 1.0),
        glow_gap: view.glow_gap.clamp(0.0, crate::GAP_MAX),
        glow_gap_soft: view.glow_gap_soft.clamp(0.0, crate::GLOW_GAP_SOFT_MAX),
        glow_gap_shape: view.glow_gap_shape.clamp(0.0, 1.0),
        glow_gap_depth: view.glow_gap_depth.clamp(0.0, 1.0),
        glow_centre: view.glow_centre.clamp(0.0, 1.0),
        glow_spread: view.glow_spread.clamp(0.0, 1.0),
        // A row per node, so a scene nothing has carried still reads one strip
        // row per node — the shell's pass hands out rows of its own and raises
        // this to their high-water mark.
        glow_rows: nodes_len,
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
pub(crate) fn derive_grid(
    view: &ViewConfig,
    window: &DrawnWindow,
    nodes: &[NodeInstance],
    ground: Vec4,
) -> Vec<EdgeInstance> {
    let inset = view.spacing * NODE_RADIUS_FACTOR * view.grid_inset.max(0.0);
    // The lattice at rest, handed in already resolved — the same colour, to
    // the byte, that a node's rings stand on where nothing is lit. OPAQUE, and
    // that is what makes the claim true rather than nearly true: `strength` is
    // the line's own opacity and the shader premultiplies by it, so a line
    // carrying an alpha of its own would land on a blend of the ground and
    // whatever happened to be behind it, which is a different grey per
    // background and none of them this one.
    //
    // How FAINT the structure is, which an alpha of its own is the other way
    // to say, is the Ground bar's to say instead — and it says it for the
    // rings in the same breath, which a per-line alpha cannot. Dialled to the
    // panel's own `L*` the whole at-rest picture disappears together.
    let base = ground;
    // `nodes` is exactly `window.positions()` in order, so a neighbor's index
    // is plain offset arithmetic — no per-frame HashMap build and no hashing
    // per lookup. `index_of` is that arithmetic and owns the bounds check with
    // it, which is what keeps an out-of-range step on one axis from aliasing
    // onto a different node's slot.
    let node_at = |p: LatticePos| -> Option<&NodeInstance> {
        window.index_of(p).and_then(|i| nodes.get(i))
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
            // A sevens link is dashed and an in-sheet line is not, and that
            // is structural rather than a style: the dash is the whole of
            // what tells a depth link from a line drawn within one sheet.
            let dashed = along_sevens;

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
                    (p.sevens + 1..=window.max.sevens, view.center_sevens..=p.sevens)
                } else {
                    (window.min.sevens..=p.sevens, p.sevens + 1..=view.center_sevens)
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
                // The lattice's own ground, always. A lit link is the same
                // structural line as a resting one, merely arrived — it says
                // WHERE a note hangs from, not what the note is, and the
                // note's color is already on the node at each end. Taking the
                // note's hue makes the chain read as a third sounding thing
                // strung between two others.
                color: base,
                // From this line's resting level up to the ground itself. A
                // home-sheet line rests there already (`idle` is the ground's
                // own alpha, 1) and never lights, so it is simply the ground;
                // a sevens link rests at nothing and this is its fade IN, as
                // the note it hangs arrives. Both ends of the lerp are the one
                // colour, so what a link fades between is invisible and the
                // ground, not two brightnesses of it.
                strength: idle + (1.0 - idle) * lit,
                dashed,
            });
        }
    }
    grid
}
