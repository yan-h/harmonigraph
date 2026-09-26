//! Per-frame scene derivation: turns the note tracker + tuning into the
//! geometry and trail history. [`crate::NodeMotion`] supplies animation and
//! final marker/ring visibility after the audio measurement.

use crate::camera::Camera;
use crate::color::pitch_ramp_lut;
use crate::octaves::octave_layout;
use crate::trail::TrailField;
use crate::view::{finite_or, size, DrawnWindow, FrameParams, ViewConfig};
use crate::{
    lattice_to_world, GlowStep, NodeInstance, PlusInstance, Scene, SpectralPaint,
    NODE_RADIUS_FACTOR, OCTAVE_SLOTS, PLUS_SIZE_MAX, PLUS_WIDTH_PER_LABEL_SCALE, SCALE_BAR_RANGE,
};
use glam::Vec4;
use harmonigraph_core::{LatticePos, NoteTracker, Tuning};

/// Build geometry and history; [`crate::NodeMotion::step`] supplies carried
/// animation and secondary visibility after the spectral measurement.
///
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
) -> Scene {
    let mut nodes = Vec::with_capacity(window.count());
    // Kept parallel to `nodes` for the trail, which matches remembered
    // pitches against every node afterwards and would otherwise have to
    // recompute each node's pitch class to do it.
    let mut node_pcs = Vec::with_capacity(window.count());
    let center = view.center();
    // The ground both of a node's rings stand on where nothing is lit.
    let ground = crate::grey_of_lightness(view.lattice_ground_lightness());
    // Sanitized once, outside the node loop. Capped at 1: this axis makes
    // off-sheet nodes SMALLER, never larger, so the home sheet stays the
    // biggest thing on screen (see `ViewConfig::sevens_size`). The floor
    // keeps a sheet from collapsing to an invisible speck at extent 4, and is
    // where a value that is not a number lands as well — the clamp alone is no
    // guard against one, and this factor is raised to the sheet count, so a
    // NaN here is every off-sheet node drawn at no size at all.
    let sevens_size = finite_or(view.sevens_size, 0.15).clamp(0.15, 1.0);
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

    for pos in window.positions() {
        let node_pc = tuning.pitch_class(pos);
        let node_cents = node_pc.to_cents();
        // World positions are relative to the window center, keeping the
        // displayed region under the camera wherever the window pans.
        let centered = pos - center;
        let world_pos = lattice_to_world(centered);

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
            (sevens_size.powi(sheets as i32), wrapped_cents(node_pc, tuning.pitch_class(namesake)))
        };
        nodes.push(NodeInstance {
            lattice_pos: pos,
            world_pos,
            activation: 0.0,
            departing: false,
            slice_progress: [1.0; OCTAVE_SLOTS],
            thickness: [1.0; OCTAVE_SLOTS],
            octaves: [0.0; OCTAVE_SLOTS],
            hovered: hovered == Some(pos),
            on_home: pos.sevens == view.center_sevens,
            scale,
            comma,
            cents: node_cents,
            melody_slots: 0,
            bass_slots: 0,
            melody_level: 0.0,
            bass_level: 0.0,
            melody_color: Vec4::ZERO,
            bass_color: Vec4::ZERO,
            // Nothing has been measured yet, so nothing can be held back: the
            // audio channel arrives empty here and `crate::NodeMotion::step` is
            // what answers this once the shell's fold has filled it.
            audio_ring: 1.0,
            // Stable snapshot row; motion supplies current ink, then the
            // shell's glow pass carries its brightness and row ownership.
            glow: GlowStep { incarnation: 0, level: 0.0, row: nodes.len() as u32 },
            trail: 0.0,
        });
        node_pcs.push(node_pc);
    }

    // Writes `trail` and nothing else (see `trail`), so nothing downstream
    // that reads "is sounding" can pick a memory up by mistake, whatever
    // order these run in. The label layer is the only reader, and it draws no
    // shape.
    if let Some(field) = TrailField::build(tracker.history(), view) {
        field.apply(&mut nodes, &node_pcs, tuning);
    }

    let nodes_len = nodes.len() as u32;

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
        node_radius: NODE_RADIUS_FACTOR,
        note_animation: view.note_animation.sanitized(),
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
        pluses: Vec::new(),
        plus_half_width: derive_plus_half_width(view),
        plus_taper_start: derive_plus_taper_start(view),
        mark_thickness: rings.mark_thickness,
        background: crate::skin::picture_color(),
        pitch_lut: pitch_ramp_lut(view.pitch_gradient),
        pitch_lut_spacing: crate::LutSpacing::of(view.pitch_gradient),
        darkest_pitch: frame.darkest_pitch,
        brightest_pitch: frame.brightest_pitch,
        // Repaired but not bounded: the renderer owns the range and
        // deliberately keeps it wider than the bar (`RENDER_SCALE_RANGE`), so a
        // range imposed here would narrow what a shell is allowed to ask for.
        // What the renderer's own clamp cannot do is catch a NaN, so this hands
        // it a real number and leaves the range where it is.
        render_scale: finite_or(view.render_scale, 1.0),
        bloom_strength: view.note_bloom_strength(),
        // Clamped here as well as in `sanitize`, for the shells that never come
        // through that door: reach sizes the halo's analytic span and its CPU
        // culling bound, which must describe the same supported range. Through
        // `finite_or` because a clamp is no guard against a NaN, and onto each
        // bar's low end, which for both of these is the halo switched off.
        glow_reach: finite_or(view.glow_reach, 0.0).clamp(0.0, crate::GLOW_REACH_MAX),
        glow_strength: finite_or(view.glow_strength, 0.0).clamp(0.0, crate::GLOW_STRENGTH_MAX),
        glow_curve: view.glow_curve.sanitized(),
        // Every Shadow group on the same footing, a bar's range rather than a
        // billboard's: every caster's quad is grown by its group's width, so a
        // number from outside the bar is a quad nothing can fill.
        shadow: view.shadow.clamped(),
        glow_wash: finite_or(view.glow_wash, 0.0).clamp(0.0, 1.0),
        // The shader divides each marker's world radius by this fixed unit
        // to recover the arm its bar was dialled at.
        marker_unit: marker_world(1.0),
        glow_blend: finite_or(view.glow_blend, 0.0).clamp(0.0, 1.0),
        // Shells may bypass `sanitize`; a mix factor outside this range would
        // extrapolate beyond the two glow treatments instead of blending them,
        // and one that is not a number would leave every blended value NaN. 0
        // is one END of the mix rather than an off position — the low bound,
        // for want of a reading here that is more neutral than another.
        glow_accumulation: finite_or(view.glow_accumulation, 0.0).clamp(0.0, 1.0),
        // A row per node, so a scene nothing has carried still reads one strip
        // row per node — the shell's pass hands out rows of its own and raises
        // this to their high-water mark.
        glow_rows: nodes_len,
        glow_timing: None,
        atmosphere: view.atmosphere.sanitized(),
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

/// Half an arm's thickness, as a share of the arm's length (see
/// [`Scene::plus_half_width`]).
///
/// The width is a LENGTH independent of the arm, set by the label scale
/// ([`PLUS_WIDTH_PER_LABEL_SCALE`]), so a long hairline and a short block are
/// both askable. The shader wants the PROPORTION, its uv being the arm's own
/// units. This is the one place that conversion happens, and the one place the
/// cross filling its own square is decided.
pub(crate) fn derive_plus_half_width(view: &ViewConfig) -> f32 {
    let arm = size(view.plus_arm, PLUS_SIZE_MAX);
    // An arm of 0 draws no markers at all, so this is only ever asked of one
    // with length — answer a proportion the shader can use rather than divide
    // by nothing, and leave the emptiness to `derive_pluses`.
    if arm <= 0.0 {
        return 0.0;
    }
    // Half, because the constant is the WHOLE thickness across an arm and the
    // shader measures out from the arm's centre line. `size` because a shell
    // that skips `sanitize` can hand over a NaN scale.
    let scale = size(view.label_scale, *SCALE_BAR_RANGE.end());
    let half = PLUS_WIDTH_PER_LABEL_SCALE * scale * 0.5;
    // At 1 the cross has filled its own square: every fragment inside the quad
    // is inside one arm or the other, and a wider one has nowhere left to
    // spread. A short arm at a big label scale gets there. Clamped rather than
    // left to the shader so the square is a stated end rather than whatever a
    // distance field happens to do past it.
    (half / arm).clamp(0.0, 1.0)
}

/// Closest a taper's start may come to the arm's tip.
///
/// The shader reads this as the low end of a `smoothstep`, and a span of zero
/// width there has no answer — so a square end is a taper that finishes within
/// a thousandth of the tip rather than exactly at it. At the sizes a marker is
/// ever drawn at that thousandth is a small fraction of one pixel, and the
/// screen-constant band the arm's end is cut with is wider than it by two
/// orders of magnitude, so what it costs the picture is nothing at all.
const TAPER_START_MAX: f32 = 0.999;

/// Where a plus's arms stop being solid, as a share of one arm's length (see
/// [`Scene::plus_taper_start`]).
///
/// The view keeps the taper as a WIDTH beside the reach, because that is the
/// pair a two-handle bar sets and the pair that lets a long arm be crisp; the
/// shader wants the POINT on an axis whose 1 is the tip. This is the one place
/// that conversion happens.
pub(crate) fn derive_plus_taper_start(view: &ViewConfig) -> f32 {
    // `size` and not a bare `clamp`, and this is the site where the difference
    // is a CRASH rather than a wrong picture: a NaN reach survives `clamp`,
    // survives `reach <= 0.0`, and then becomes the `max` of the taper's own
    // clamp below — which panics on a NaN bound. `sanitize` repairs the arm at
    // the blob's door, so what reaches this is every shell that has no such
    // door: the offline renderer's layout, a take replay, the harness.
    let reach = size(view.plus_arm, PLUS_SIZE_MAX);
    // A reach of 0 draws no markers at all, so this is only ever asked of an
    // arm that has length — answer the square end rather than dividing by
    // nothing, and leave the emptiness to `derive_pluses`.
    if reach <= 0.0 {
        return TAPER_START_MAX;
    }
    let taper = size(view.plus_taper, reach);
    ((reach - taper) / reach).clamp(0.0, TAPER_START_MAX)
}

/// One quad-uv length of the home sheet, as a world length.
///
/// uv 1 is 1.8 node radii out (`node_vertex` in lattice.wgsl), so the bars a
/// marker is dialled on — the same units every ring radius on a node is in —
/// resolve here, once, rather than the shader carrying a second copy of the
/// convention for one more layer. The home sheet has no scale of its own, which
/// is the sheet every marker stands on ([`derive_pluses`]).
fn marker_world(uv: f32) -> f32 {
    NODE_RADIUS_FACTOR * 1.8 * uv
}

/// The lattice's resting picture: idle positions draw no disc, so a small
/// cross stands at each one and carries the structure instead. Only the home
/// (center) sheet gets them.
///
/// A marker at each position rather than a line between them, and the
/// difference is what the picture claims. Lines draw the INTERVALS — one
/// segment per unit step along a prime axis — so the lattice is a mesh and a
/// note lands on a junction in it. These draw the POSITIONS and nothing else:
/// what runs between two of them is left to the eye, which reads the rows and
/// columns off a regular field anyway, and the ink a line between every pair
/// would cost goes to the notes instead.
///
/// A CROSS is that argument at its sharpest: it is exactly what a pair of
/// gridlines draws where they meet, so it keeps every junction a mesh would
/// have and still spends no ink getting from one junction to the next.
///
/// Off-sheet positions stay unmarked. That is the whole of what makes one
/// sheet the ground: a 7-limit note sounding off
/// it floats over the marker field rather than standing in it, and the size
/// it draws at ([`NodeInstance::scale`]) is what says how far off it has
/// gone.
///
/// A NAMED position is unmarked too ([`NodeInstance::name_level`]). Both a
/// marker and a name say "a position is here", and the name says which one, so
/// the marker behind it is the weaker of two claims on the same spot and the
/// picture is cleaner without it. Under [`NoteNames::All`](crate::NoteNames::All)
/// that is every node on screen and the field disappears whole — which is the
/// mode working as it reads: names ARE the lattice there, and a marker only
/// ever stands in for one.
///
/// The name takes the marker by DEGREE, not by decree: a marker's opacity is
/// what is left over from the name above it. Under
/// [`Played`](crate::NoteNames::Played) a name is drawn at the node's own
/// activation, so a released note's name spends the end of its fade invisible —
/// and a marker that waited for the name to be gone ENTIRELY would leave the
/// position empty for that stretch and then pop in at full opacity. The
/// complement makes the two one crossing surface.
pub(crate) fn derive_pluses(
    view: &ViewConfig,
    nodes: &[NodeInstance],
    ink: Vec4,
) -> Vec<PlusInstance> {
    let radius = marker_world(size(view.plus_arm, PLUS_SIZE_MAX));
    // 0 takes the markers away, and with them everything a resting lattice
    // draws but the node rings. Skipping the instances is the same picture the
    // shader would discard to, one draw earlier.
    //
    // An arm that is not a real number takes them away through this SAME test,
    // and that is a property of `size` rather than of the line below: it is the
    // only door into the radius and it answers every non-finite value with 0 —
    // the repair `sanitize` spends at the blob's door, spent again on the
    // picture's side for the shells that never come through it.
    //
    // So there is no second branch to write, and a NaN test here would be one
    // nothing can reach. What a later factor owes is the door, not the test: a
    // NaN arriving at this line would answer no to `<= 0.0` the way it answers
    // no to every comparison, and ship the whole field sized NaN — a quad the
    // shader cannot draw, the lattice's resting structure gone, nothing on
    // screen saying why. Multiply something in that `size` has not been over
    // and the repair is owed at that factor.
    if radius <= 0.0 {
        return Vec::new();
    }
    // The markers' own grey, handed in already resolved from the Marker ink
    // bar (`ViewConfig::marker_ink_lightness`). OPAQUE, and that is what makes
    // the bar's number the grey on screen rather than nearly it: `strength` is
    // the marker's own opacity and the shader premultiplies by it, so a marker
    // carrying a standing alpha of its own would land on a blend of that grey
    // and whatever happened to be behind it — a different colour per
    // background, and none of them the one asked for.
    //
    // How FAINT the field is, which a standing alpha is the other way to say,
    // is that bar's to say instead. A brightness reads against the pane and
    // against the ground the node rings stand on, both of which are colours a
    // person can see; an alpha reads against whatever is behind, which here is
    // sometimes a halo.
    //
    // What does move it is a NAME standing over the position, and only while
    // that name is on its way in or out: a marker gets what the name leaves, so
    // the two hand the position over without it going empty or being held twice
    // (`name_level`). Fully named is fully gone, and the instance is dropped
    // rather than shipped at zero — a marker nothing can see is a draw nothing
    // needs.
    //
    // A name and NOTHING else, which is the whole rule: the cross disappears if
    // and only if a name is present. A note reaches it through the name rather
    // than beside it, and does so under every Show mode — `name_level` is
    // `activation.max(resting)`, so a sounding note is named at its own
    // activation even under `Played`, where nothing rests. Asking the note a
    // second time here would therefore change one case only, the one where
    // there are no names to be present: with the Note names switched off a
    // sounding note would take a marker that no name is taking, which is the
    // rule read backwards.
    //
    // So an analyzer ring moves this by nothing, having no name to put over a
    // position. Nor does the LIGHT standing over one, and that is the same rule
    // read on the marker's SHADOW: the share of the shadow a cross casts rides
    // this one number with the ink (`PlusInstance::strength`), so the two fade
    // in together as the name hands the position back. A shadow closed on the
    // light instead is a cross arriving whole with nothing under it and a
    // shadow easing in seconds behind it, on a clock nothing on screen
    // explains.
    nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.on_home)
        .filter_map(|(node, n)| {
            let clear = 1.0 - n.name_level(view);
            (clear > 0.0).then(|| {
                let strength = ink.w * clear;
                PlusInstance { node, pos: n.world_pos, radius, color: ink, strength }
            })
        })
        .collect()
}
