//! Where each octave indicator sits around a node: the pitch axis the wheel
//! is, how wide each octave of it is, and which octaves a node draws.
//!
//! **The wheel is a pitch axis, and it is the same axis on every node.** One
//! monotone map takes an absolute MIDI pitch to an angle: middle C straight
//! up, rising pitch clockwise, one full turn covering the window the Range
//! setting names. An indicator is drawn at the angle of the pitch it stands
//! for — so where a node's indicators sit says which pitches those octaves
//! ARE, and two nodes' indicators for the same octave sit at different
//! angles exactly as their pitches differ.
//!
//! **The Range is a count of indicators, and the window is what holds them.**
//! At ±`span` a node draws the `2 * span + 1` octaves from `span` below
//! middle C's to `span` above — the same octave NUMBERS on every node
//! whatever its pitch class, so ±2 is C2..C6 in this crate's numbering
//! (middle C = C4; the UI and the DAW spell the same five C1..C5, in
//! Bitwig's, which is what the Range row reads). Each is one whole
//! octave of the axis, centered on its own pitch, so the window has to be
//! `2 * span + 1` octaves wide to hold them: half an octave past the
//! outermost indicators' own pitches on either side. They then tile the turn
//! exactly, each meeting its neighbours at the boundary they share.
//!
//! **The bottom is both ends of the window at once, and the axis runs
//! THROUGH it.** The lowest pitch the window shows and the highest land on
//! the same point, straight down, on every node whatever its pitch class —
//! which is the thing a per-node rotation of the wheel cannot give. On a
//! node whose pitch class is not C the highest indicator runs past the high
//! end and comes round to the low one, since that is the same point of the
//! circle. The axis's first and last octaves are the same distance from
//! middle C and so exactly the same width, which is what makes continuing
//! past the seam and wrapping round it the same angle.
//!
//! The map is computed here, on the CPU, and handed to the shader as a table
//! of boundary angles — the alternative is accumulating the same widths per
//! pixel per sector, which is the same arithmetic done a few million times a
//! frame for a value that changes when a setting does.

use std::f32::consts::{FRAC_PI_2, TAU};

/// Octave indicator slots: MIDI octaves -1..=9, so `slot = octave + 1` and
/// middle C (octave 4, MIDI 60) is slot 5. Slot `s` is the octave whose C is
/// MIDI `12 * s`. Eleven covers the whole MIDI range; the renderer packs one
/// byte per slot into 3 words and asserts the count fits.
pub const OCTAVE_SLOTS: usize = 11;

/// Slot of middle C's octave, and the middle of every window: the Range
/// setting counts octaves out from here, so this slot's indicator is drawn on
/// every node at every setting.
pub const MIDDLE_C_SLOT: usize = 5;

/// Narrowest Range: 2 octaves either side of middle C's, so 5 indicators.
pub const MIN_OCTAVE_SPAN: u32 = 2;
/// Widest Range: 5 either side, so 11 indicators — every octave MIDI has.
pub const MAX_OCTAVE_SPAN: u32 = 5;

/// Indicators the widest Range draws, which is also the octaves its window
/// spans — they are one to one.
const MAX_INDICATORS: usize = 2 * MAX_OCTAVE_SPAN as usize + 1;

// The widest Range has to fit the fixed-size tables it is written into: one
// slot per indicator, and one boundary angle per octave of the window plus
// the closing one. Raising MAX_OCTAVE_SPAN alone would run `octave_layout`
// off the end of `bounds`, a runtime panic in the render path; the renderer's
// own ceiling on OCTAVE_SLOTS is a build error, and this makes the pair fail
// the same way.
const _: () = assert!(MAX_INDICATORS <= OCTAVE_SLOTS);
// Fitting is not enough: the indicators are CENTERED on middle C's slot, so
// the widest Range has to reach both ends from there. Counting down off slot
// 0 is a u32 underflow in `slot_range` — a panic in that same render path,
// and one the count check above passes straight over, since a wider table
// would leave room for the indicators without moving where they start.
const _: () = assert!(MAX_OCTAVE_SPAN as usize <= MIDDLE_C_SLOT);
const _: () = assert!(MIDDLE_C_SLOT + MAX_OCTAVE_SPAN as usize <= OCTAVE_SLOTS - 1);

/// Ceiling on the taper amount. At 1 the outermost octave would have no
/// width at all, which is a window that claims to show a pitch and doesn't;
/// 0.9 leaves it a tenth of an even slice, still a sliver but a visible
/// one — 3 degrees at the widest Range, 7 at the narrowest.
pub const MAX_TAPER_AMOUNT: f32 = 0.9;

/// Semitones to the octave, as a float: this module is all pitch arithmetic
/// and the conversions read better named.
const SEMIS: f32 = 12.0;

/// How far the Shape setting can bend the taper either way, as an exponent:
/// the ends of the bar are `x^(1/4)` and `x^4`, and its middle is the
/// straight ramp `x`. Four is where more travel stops buying a different
/// picture — at the sharp end the octave beside middle C already keeps under
/// a third of the width the middle one takes over an even slice, and at the
/// flat end every octave inside the edges is within a fifth of the middle.
const SHAPE_EXTREME: f32 = 4.0;

/// The exponent the distance is raised to, from a Shape setting of 0..1.
/// Logarithmic, so the bar's middle is the straight ramp (exponent 1) and
/// the two halves are mirror images of each other rather than one being a
/// squashed version of the other.
fn shape_exponent(shape: f32) -> f32 {
    SHAPE_EXTREME.powf(2.0 * shape.clamp(0.0, 1.0) - 1.0)
}

/// How wide one octave of the axis comes out, as a MULTIPLE OF AN EVEN
/// SLICE: 1 is the width it would have under no taper at all, and the
/// widths across a Range add up to the octave count, which is what makes
/// that unit mean something. `fall` is how far this octave is along the
/// shape curve (0 at middle C, 1 at the edge of the Range) and `lift` is
/// what the octaves inside the edge share out (see [`octave_layout`]).
///
/// A taper bends the pitch axis without breaking it: the map stays monotone
/// and stays linear WITHIN each octave, so an indicator still sits on its
/// own pitch and still spans exactly its own octave. What changes is how
/// many degrees an octave is worth, which is a statement about emphasis
/// rather than about pitch.
///
/// Two knobs that mean two different things, which is the whole reason this
/// is a pair of bars and not a list of named curves:
///
/// - The AMOUNT sets the EDGE slices, and nothing else does: they come out
///   `1 - amount` of an even slice at every shape and every Range. That is a
///   size on screen rather than a ratio against another octave, which
///   matters because the octave a ratio would be against is the one the
///   shape moves the most — pinning the edge's RELATIVE weight instead let
///   dragging Shape toward the plateau, which widens everything but the
///   middle, take degrees away from the edge slices.
/// - The SHAPE says where the width the edges give up lands, through the
///   exponent `p` (see [`shape_exponent`]). Left of the bar's middle, `p` is
///   under 1 and the fall happens at once: the octave next to middle C gives
///   up most of what the edge one does and the outer ones flatten off, so
///   the middle keeps nearly all of the lift — a spotlight. Right of it `p`
///   is over 1 and the fall is held back to the extremes: the octaves either
///   side of middle C stay nearly as wide as it is, a plateau rather than a
///   gradient. Dragging it moves degrees between the middle octave and the
///   ones around it, and only between those — the edges hold still.
///
/// An even axis is `amount` 0, so it is where the bar starts rather than a
/// mode beside it — and the shape is inert there, which is exactly what "no
/// taper" should mean.
fn octave_width(fall: f32, amount: f32, lift: f32) -> f32 {
    (1.0 - amount.clamp(0.0, MAX_TAPER_AMOUNT)) + lift * fall.clamp(0.0, 1.0)
}

/// The pitch axis the octave indicators are drawn on, ready for the shader.
///
/// Computed once per frame from the view settings, not per node: every node
/// reads the SAME axis, which is what makes an indicator's angle mean an
/// absolute pitch rather than a position in that node's own private ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OctaveLayout {
    /// MIDI pitch at the seam approached from the LEFT — the lowest pitch
    /// the window shows. [`high_pitch`](Self::high_pitch) is the same point
    /// on the circle, come round the other way.
    pub low_pitch: f32,
    /// Octaves the window spans: `2 * span + 1`, one per indicator. Its ends
    /// are the F♯s half an octave outside the outermost indicators' Cs, so
    /// each of its octaves is CENTERED on a C and the axis's own octaves are
    /// exactly a C node's indicators.
    pub octaves: u32,
    /// Angle of each octave boundary, in radians, walking CLOCKWISE (the
    /// direction pitch rises) from the seam: `bounds[0]` is the bottom at
    /// `low_pitch`, and `bounds[octaves]` is a full turn on — the same seam,
    /// at the window's high end. Entries past `octaves` repeat the last so a
    /// stale index cannot produce a wild angle.
    pub bounds: [f32; OCTAVE_SLOTS + 1],
}

impl Default for OctaveLayout {
    fn default() -> Self {
        octave_layout(DEFAULT_OCTAVE_SPAN, 0.0, DEFAULT_TAPER_SHAPE)
    }
}

/// Shape the view starts at: the straight ramp, which is the middle of the
/// bar and the one shape that is neither a spotlight nor a plateau. It shows
/// nothing until the Amount leaves 0.
pub const DEFAULT_TAPER_SHAPE: f32 = 0.5;

/// Range the view starts at: 4 octaves either side of middle C's, so 9
/// indicators — C0..C8 in this crate's numbering (middle C = C4; the UI
/// spells the same nine C-1..C7, in Bitwig's). They reach past both ends of
/// any keyboard part, and at 9 octaves to the turn an octave is worth 40
/// degrees, which is wide enough to read at a glance.
pub const DEFAULT_OCTAVE_SPAN: u32 = 4;

/// The pitch axis for a Range of `span` octaves either side of middle C's,
/// tapered by `amount` in the shape `shape` (see [`taper_weight`]).
///
/// The seam is where the walk STARTS and a full turn is what it covers,
/// which is the whole of why the window's two ends meet at the bottom under
/// any settings: the invariant is structural rather than a property of the
/// widths that a new formula could break. Middle C lands straight up because
/// the widths depend only on the distance from it and are therefore
/// symmetric about it — half the circle either side.
pub fn octave_layout(span: u32, amount: f32, shape: f32) -> OctaveLayout {
    let span = span.clamp(MIN_OCTAVE_SPAN, MAX_OCTAVE_SPAN);
    let octaves = 2 * span + 1;
    let n = span as f32;

    // How far each octave's own C is from middle C, normalized by the span,
    // so the taper's shape is the same picture at every Range and only its
    // resolution changes. The middle octave is at distance 0 and the two end
    // ones at 1, which is what makes them equal — and that equality is what
    // lets the highest indicator run off the top of the axis and come round
    // the seam at the right width.
    //
    // What the shape bends is how much each octave gives up on the way out,
    // `fall`, which is 0 at middle C and 1 at the edge whatever the exponent.
    let p = shape_exponent(shape);
    let mut fall = [0f32; OCTAVE_SLOTS];
    let mut fall_total = 0.0;
    for (j, f) in fall.iter_mut().take(octaves as usize).enumerate() {
        *f = 1.0 - ((j as f32 - n).abs() / n).powf(p);
        fall_total += *f;
    }

    // The widths, in multiples of an EVEN slice (see `octave_width`). The
    // lift is what the octaves inside the edge share out, and it is not a
    // setting: the widths have to add up to the circle, which in these units
    // is exactly `octaves` even slices, and that pins it.
    let lift = octaves as f32 * amount.clamp(0.0, MAX_TAPER_AMOUNT) / fall_total.max(1e-6);
    let mut weights = [0f32; OCTAVE_SLOTS];
    let mut total = 0.0;
    for (j, w) in weights.iter_mut().take(octaves as usize).enumerate() {
        *w = octave_width(fall[j], amount, lift);
        total += *w;
    }

    // Clockwise is the direction pitch rises (uv.y is up, so the angle
    // decreases), which is why the walk subtracts.
    let mut bounds = [-FRAC_PI_2; OCTAVE_SLOTS + 1];
    let mut acc = 0.0;
    for j in 0..octaves as usize {
        acc += weights[j];
        bounds[j + 1] = -FRAC_PI_2 - TAU * (acc / total);
    }
    for j in octaves as usize + 1..bounds.len() {
        bounds[j] = bounds[octaves as usize];
    }

    OctaveLayout {
        low_pitch: 60.0 - SEMIS * (n + 0.5),
        octaves,
        bounds,
    }
}

impl OctaveLayout {
    /// The highest MIDI pitch the window shows — the seam again, approached
    /// from the right.
    pub fn high_pitch(&self) -> f32 {
        self.low_pitch + SEMIS * self.octaves as f32
    }

    /// Where MIDI pitch `pitch` sits on the wheel, in radians. Linear within
    /// each octave and monotone across them, so an interval reads as an
    /// angle.
    ///
    /// Past the window's high end it CONTINUES rather than clamping, at the
    /// last octave's scale: the highest indicator on a node whose pitch class
    /// is not C runs up to an octave past that end, and continuing is the
    /// same angle as wrapping round the seam into the window's first octave,
    /// which is the same width. Below the low end there is nothing to
    /// continue for — no indicator reaches under it — so that side clamps.
    pub fn angle(&self, pitch: f32) -> f32 {
        let x = ((pitch - self.low_pitch) / SEMIS).max(0.0);
        let j = (x as usize).min(self.octaves as usize - 1);
        self.bounds[j] + (self.bounds[j + 1] - self.bounds[j]) * (x - j as f32)
    }

    /// MIDI pitch of octave slot `slot` on a node whose pitch class is
    /// `cents` (0..1200): the pitch that indicator stands for, and the pitch
    /// it is centered on.
    pub fn slot_pitch(&self, slot: u32, cents: f32) -> f32 {
        slot as f32 * SEMIS + cents / 100.0
    }

    /// The two edge angles of slot `slot`'s indicator on a node whose pitch
    /// class is `cents` — its octave's two ends, read off the axis, the
    /// counter-clockwise one first. The shader's `oct_sector` is this.
    pub fn sector(&self, slot: u32, cents: f32) -> (f32, f32) {
        let pitch = self.slot_pitch(slot, cents);
        (self.angle(pitch - 6.0), self.angle(pitch + 6.0))
    }

    /// The slots every node draws, inclusive: `span` either side of middle
    /// C's. The same octaves on every node whatever its pitch class — a
    /// slot is a MIDI octave, so the Range names octave NUMBERS and each
    /// node's pitch class only says where in the turn its own land.
    ///
    /// Notes outside it fold onto the nearest end (see `derive_scene`), so
    /// this is also what a voice's octave is clamped into.
    pub fn slot_range(&self) -> (u32, u32) {
        let span = (self.octaves - 1) / 2;
        (MIDDLE_C_SLOT as u32 - span, MIDDLE_C_SLOT as u32 + span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    /// The Shape bar at both ends, at its middle, and either side of it: the
    /// sharpest spotlight, the straight ramp, and the flattest plateau.
    const SHAPES: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

    /// Every span, formula and amount, against pitch classes that put an
    /// indicator's edge exactly on the seam (C), well clear of it, and just
    /// short of it — the grid the invariants below all run over.
    fn every_case() -> impl Iterator<Item = (OctaveLayout, f32, String)> {
        (MIN_OCTAVE_SPAN..=MAX_OCTAVE_SPAN).flat_map(|span| {
            SHAPES.iter().flat_map(move |&shape| {
                [0.0f32, 0.35, 0.9].iter().flat_map(move |&amount| {
                    [0.0f32, 350.0, 700.0, 1150.0].iter().map(move |&cents| {
                        (
                            octave_layout(span, amount, shape),
                            cents,
                            format!("span {span}, amount {amount} shape {shape}, {cents}c"),
                        )
                    })
                })
            })
        })
    }

    /// The seam: the window's two ends are one point, and it is the bottom
    /// of the node.
    #[test]
    fn the_windows_ends_meet_at_the_bottom() {
        for (l, _, case) in every_case() {
            assert!((l.angle(l.low_pitch) - -FRAC_PI_2).abs() < 1e-5, "{case}");
            let high = l.angle(l.high_pitch());
            assert!((high - (-FRAC_PI_2 - TAU)).abs() < 1e-4, "{case}: {high}");
        }
    }

    /// Middle C is straight up — on the axis itself, so on every node.
    #[test]
    fn middle_c_is_straight_up() {
        for (l, _, case) in every_case() {
            let up = l.angle(60.0);
            assert!((up - (-FRAC_PI_2 - PI)).abs() < 1e-4, "{case}: {up}");
        }
    }

    /// What "faithful" means, as an assertion: the map is strictly falling
    /// in pitch, and equal intervals WITHIN one octave subtend equal angles,
    /// so an indicator sits on its pitch rather than near it.
    #[test]
    fn the_axis_is_monotone_and_linear_inside_an_octave() {
        for (l, _, case) in every_case() {
            let mut previous = l.angle(l.low_pitch);
            let mut pitch = l.low_pitch;
            while pitch < l.high_pitch() - 0.25 {
                pitch += 0.5;
                let a = l.angle(pitch);
                assert!(a < previous, "{case}: not falling at {pitch}");
                previous = a;
            }
            // Three points evenly spaced in pitch inside ONE octave come out
            // evenly spaced in angle.
            let base = l.low_pitch + SEMIS * (l.octaves / 2) as f32;
            let (a, b, c) = (l.angle(base), l.angle(base + 4.0), l.angle(base + 8.0));
            assert!(((a - b) - (b - c)).abs() < 1e-4, "{case}: uneven inside an octave");
        }
    }

    /// Past the window's high end the axis keeps going, and where it goes is
    /// exactly where wrapping round the seam would put it — the octave up
    /// there and the one at the bottom are equally far from middle C, so
    /// they are equally wide. That is what carries the highest indicator on
    /// a node that is not a C across the bottom in one piece.
    #[test]
    fn the_axis_continues_past_the_seam_where_wrapping_would_land() {
        for (l, _, case) in every_case() {
            for semitones in 0..=12 {
                let d = semitones as f32;
                let past = l.angle(l.high_pitch() + d);
                let round = l.angle(l.low_pitch + d) - TAU;
                let off = (past - round).abs();
                assert!(off < 1e-4, "{case}: {d} past the seam: {past} vs {round}");
            }
        }
    }

    /// The Range names octave NUMBERS, and every node draws all of them: the
    /// set is `span` slots either side of middle C's whatever the node's
    /// pitch class, so ±2 is slots 3..7 on a C node and on every other.
    #[test]
    fn every_node_draws_the_octaves_the_range_names() {
        for span in MIN_OCTAVE_SPAN..=MAX_OCTAVE_SPAN {
            let l = octave_layout(span, 0.0, DEFAULT_TAPER_SHAPE);
            for cents in [0.0f32, 350.0, 700.0, 1150.0] {
                let (low, high) = l.slot_range();
                assert_eq!(
                    (low, high),
                    (MIDDLE_C_SLOT as u32 - span, MIDDLE_C_SLOT as u32 + span),
                    "span {span}, {cents}c"
                );
                assert_eq!(high - low + 1, 2 * span + 1, "span {span}: indicator count");
                // Room for all of them, with the outermost pair's own pitches
                // half an octave inside the window's ends.
                assert!(l.slot_pitch(low, cents) - 6.0 >= l.low_pitch - 1e-3);
                assert!(l.slot_pitch(high, cents) - 6.0 < l.high_pitch());
            }
        }
    }

    /// A drawn indicator is a WHOLE octave, centered on its own pitch. That
    /// is what keeps the mapping honest: an indicator cut short would
    /// misstate its octave's width, and one moved to fit would misstate its
    /// pitch.
    #[test]
    fn drawn_indicators_are_whole_octaves_on_their_own_pitch() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range();
            assert!(low <= high, "{case}: nothing drawn");
            for slot in low..=high {
                let pitch = l.slot_pitch(slot, cents);
                // Its edges are its octave's ends, read off the axis: half an
                // octave either side of its own pitch, and nothing else.
                let (e0, e1) = l.sector(slot, cents);
                assert!(e0 > e1, "{case}: slot {slot} runs backwards");
                let inside = l.angle(pitch);
                assert!(e0 > inside && inside > e1, "{case}: slot {slot} misses its own pitch");
                // Under an even axis that also puts the pitch at the
                // indicator's angular MIDDLE. A taper legitimately breaks
                // that and only that: the scale changes at each boundary, so
                // an indicator straddling one has its two halves at different
                // scales. Its edges are still exactly its octave's ends,
                // which is what "positioned by pitch" means here.
                if l.bounds[0] - l.bounds[1] == l.bounds[1] - l.bounds[2] {
                    assert!(
                        (inside - 0.5 * (e0 + e1)).abs() < 1e-4,
                        "{case}: slot {slot} is not centered on its pitch"
                    );
                }
            }
        }
    }

    /// The indicators tile the turn: each one's clockwise edge is the next
    /// one's counter-clockwise edge, and the highest one's far edge is the
    /// lowest one's near edge come round the seam. So the ring closes on
    /// every node whatever its pitch class — which is the whole point of
    /// giving the window an octave per indicator and half an octave over.
    #[test]
    fn the_indicators_tile_the_turn() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range();
            let mut total = 0.0;
            for slot in low..=high {
                let (e0, e1) = l.sector(slot, cents);
                total += e0 - e1;
                let next = if slot == high {
                    // Round the seam: the same point, one turn on.
                    l.sector(low, cents).0 - TAU
                } else {
                    l.sector(slot + 1, cents).0
                };
                assert!((e1 - next).abs() < 1e-4, "{case}: slot {slot} leaves {} rad", e1 - next);
            }
            assert!((total - TAU).abs() < 1e-4, "{case}: the ring covers {total} rad");
        }
    }

    /// Octaves shrink outward, symmetrically, at every shape — and only when
    /// an amount is asked for: the middle of the window is what carries the
    /// extra weight.
    #[test]
    fn octaves_shrink_away_from_middle_c() {
        for shape in SHAPES {
            for amount in [0.0f32, 0.6] {
                let l = octave_layout(5, amount, shape);
                let width = |j: usize| l.bounds[j] - l.bounds[j + 1];
                // Eleven octaves, so middle C's own is the middle one, index
                // 5, and the walk outward from it either way is what shrinks.
                for j in 0..5 {
                    let (inner, outer) = (width(j + 1), width(j));
                    let case = format!("shape {shape}, amount {amount}, octave {j}");
                    if amount == 0.0 {
                        assert!((inner - outer).abs() < 1e-6, "{case}: an even axis moved");
                    } else {
                        assert!(outer < inner, "{case}: {outer} !< {inner}");
                    }
                    // Symmetric about middle C's octave.
                    assert!((width(j) - width(10 - j)).abs() < 1e-6, "{case}: lopsided");
                }
            }
        }
    }

    /// The Amount alone sets the edge slices, and the Shape leaves them
    /// exactly where they are: an edge octave is `1 - amount` of an even
    /// slice at every shape and every Range. Dragging Shape across its whole
    /// travel must not cost the outermost octaves a degree — they are the
    /// ones with the least to give.
    #[test]
    fn the_amount_alone_sets_the_edge_slices() {
        for span in MIN_OCTAVE_SPAN..=MAX_OCTAVE_SPAN {
            for amount in [0.0f32, 0.35, MAX_TAPER_AMOUNT] {
                let even = TAU / (2 * span + 1) as f32;
                for shape in SHAPES {
                    let l = octave_layout(span, amount, shape);
                    let edge = l.bounds[0] - l.bounds[1];
                    let case = format!("span {span}, amount {amount}, shape {shape}");
                    assert!(
                        (edge - (1.0 - amount) * even).abs() < 1e-4,
                        "{case}: the edge slice is {edge} rad, not {}",
                        (1.0 - amount) * even
                    );
                }
            }
        }
    }

    /// What the Shape does move, given it cannot touch the edges: degrees
    /// between middle C's octave and the ones out toward them. Every step
    /// right flattens the profile — the middle narrows and the octaves next
    /// to the edges widen — and every step left concentrates it again.
    #[test]
    fn the_shape_flattens_the_profile_between_the_edges() {
        let width = |shape: f32, j: usize| {
            let l = octave_layout(4, 0.6, shape);
            (l.bounds[j] - l.bounds[j + 1]).to_degrees()
        };
        // Nine octaves: middle C's is index 4, and index 1 is the widest one
        // the shape can still reach, just inside the pinned edge.
        let mut previous: Option<(f32, f32)> = None;
        for shape in SHAPES {
            let (middle, outer) = (width(shape, 4), width(shape, 1));
            if let Some((was_middle, was_outer)) = previous {
                assert!(middle < was_middle, "shape {shape}: the middle did not narrow");
                assert!(outer > was_outer, "shape {shape}: the outer one did not widen");
            }
            previous = Some((middle, outer));
        }
        // The bar's middle is the straight ramp: equal steps octave to octave.
        let ramp: Vec<f32> = (0..4).map(|j| width(0.5, j + 1) - width(0.5, j)).collect();
        for step in &ramp {
            assert!((step - ramp[0]).abs() < 1e-3, "the bar's middle is not a straight ramp");
        }
    }

    /// An indicator can pass a half turn but never a whole one, which is
    /// exactly the pair of facts the shader's wedge test is built on: past a
    /// half turn a wedge is the UNION of its two half-planes rather than
    /// their intersection, and at a whole turn neither reading means
    /// anything. The widest there is is middle C's own octave at the
    /// narrowest Range, the fullest amount and the sharpest shape: five
    /// octaves to the turn, the edges pinned at a tenth of an even slice and
    /// the middle taking everything they and their neighbours give up — 253
    /// degrees of the 360.
    #[test]
    fn an_indicator_can_pass_a_half_turn_but_never_a_whole_one() {
        let mut widest: f32 = 0.0;
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range();
            for slot in low..=high {
                let (e0, e1) = l.sector(slot, cents);
                assert!(e0 - e1 < TAU, "{case}: slot {slot} spans {} rad", e0 - e1);
                widest = widest.max(e0 - e1);
            }
        }
        assert!(widest > PI, "nothing passes a half turn: widest {widest} rad");
        let steepest = octave_layout(MIN_OCTAVE_SPAN, MAX_TAPER_AMOUNT, 0.0);
        let (e0, e1) = steepest.sector(MIDDLE_C_SLOT as u32, 0.0);
        assert!((widest - (e0 - e1)).abs() < 1e-5, "the widest is somewhere else: {widest}");
    }

    /// A span outside the supported range is clamped rather than producing a
    /// layout the shader's fixed-size tables cannot hold. The widest holds
    /// every octave MIDI has, so nothing folds there.
    #[test]
    fn span_is_clamped_to_the_tables() {
        assert_eq!(octave_layout(0, 0.0, 0.5).octaves, 2 * MIN_OCTAVE_SPAN + 1);
        let widest = octave_layout(99, 0.0, 0.5);
        assert_eq!(widest.octaves, 2 * MAX_OCTAVE_SPAN + 1);
        assert_eq!(widest.octaves as usize, OCTAVE_SLOTS);
        assert_eq!(widest.slot_range(), (0, OCTAVE_SLOTS as u32 - 1));
        assert_eq!(widest.low_pitch, -6.0);
        assert_eq!(widest.high_pitch(), 126.0);
    }
}
