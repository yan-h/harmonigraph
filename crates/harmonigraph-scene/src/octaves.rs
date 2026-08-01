//! Where each octave indicator sits around a node: the pitch window the wheel
//! shows, how wide each octave of it is, and which octaves a node draws.
//!
//! **The wheel is a pitch axis, and it is the same axis on every node.** One
//! monotone map takes an absolute MIDI pitch to an angle: the window's low end
//! straight down, rising pitch clockwise, one full turn covering the window
//! the Range setting names. An indicator is drawn at the angle of the pitch it
//! stands for — so where a node's indicators sit says which pitches those
//! octaves ARE, and two nodes' indicators for the same octave sit at different
//! angles exactly as their pitches differ.
//!
//! **The Range is a PITCH WINDOW, continuous at both ends.** It is not a count
//! of octaves: any low and any high pitch [`MIN_WINDOW`] apart will do, and the
//! octaves are what the window is divided BY rather than what it is measured
//! in. That is the whole reason the geometry below is what it is — a window
//! that is not a whole number of octaves cannot have every node's octaves tile
//! it (a pitch class has `floor` or `ceil` of them inside depending on where it
//! falls), so the two indicators at the ENDS reach out to the seam and the ones
//! between are exactly their own octave. Nothing is drawn twice and nothing is
//! left bare, at any window and any pitch class.
//!
//! **A node draws the octaves of ITSELF that land in the window.** So two nodes
//! can show a different count — nine and ten — when the window is not a whole
//! number of octaves, and where each indicator sits still says its pitch. (At a
//! whole number of octaves the counts agree again, which is the old fixed-span
//! wheel come back as a special case.) Notes outside the window fold onto the
//! nearest end, so a narrow Range is a way of READING the music rather than a
//! filter over it.
//!
//! **The bottom is both ends of the window at once.** The lowest pitch the
//! window shows and the highest land on the same point, straight down, on every
//! node whatever its pitch class — which is the thing a per-node rotation of the
//! wheel cannot give. The window's MIDDLE pitch is then straight up, since the
//! widths depend only on the distance from it and are therefore symmetric about
//! it; with the default window, which is centered on middle C, that is middle C
//! exactly as it reads on a keyboard.
//!
//! The map is computed here, on the CPU, and handed to the shader as a table of
//! boundary angles — the alternative is accumulating the same widths per pixel
//! per sector, which is the same arithmetic done a few million times a frame for
//! a value that changes when a setting does.

use std::f32::consts::{FRAC_PI_2, TAU};

/// Octave indicator slots: MIDI octaves -1..=9, so `slot = octave + 1` and
/// middle C (octave 4, MIDI 60) is slot 5. Slot `s` is the octave whose C is
/// MIDI `12 * s`. Eleven covers the whole MIDI range; the renderer packs one
/// byte per slot into 3 words and asserts the count fits.
pub const OCTAVE_SLOTS: usize = 11;

/// Slot of middle C's octave: the slot a note at MIDI 60 lights, and the one
/// the default window is centered on.
pub const MIDDLE_C_SLOT: usize = 5;

/// Lowest and highest pitch the Range can reach — the MIDI note numbers, so
/// every slot's C (0 to 120) is inside and the readout is a note anyone can
/// name.
pub const PITCH_FLOOR: f32 = 0.0;
/// See [`PITCH_FLOOR`].
pub const PITCH_CEIL: f32 = 127.0;

/// Narrowest window, in semitones: four octaves — C1..C5 in the DAW's
/// numbering, the register a keyboard part actually lives in.
///
/// A floor rather than a preference, and the taper is why. The width the
/// middle of the axis takes is shared out from the cells either side of it, so
/// the fewer cells there are the more of the turn one indicator can end up
/// holding. Measured, at the fullest amount and the sharpest shape, sweeping
/// the window along the axis at every pitch class: four octaves takes the
/// widest indicator to 266 degrees and five to 253, but three and a half takes
/// it to 274, three to 336, and two to 342 — closing on a whole turn, where a
/// wedge is no longer a wedge and the shader's two-half-plane test has nothing
/// to say. See [`an_indicator_can_pass_a_half_turn_but_never_a_whole_one`](self).
pub const MIN_WINDOW: f32 = 48.0;

/// The window a fresh view starts on: C0..C8 in this crate's numbering (middle
/// C = C4; the UI spells the same C-1..C7, in Bitwig's), centered on middle C
/// and reaching half an octave past the outermost C either way. Past both ends
/// of any keyboard part, and at nine octaves to the turn an octave is worth 40
/// degrees, which reads at a glance.
pub const DEFAULT_WINDOW_LOW: f32 = 6.0;
/// See [`DEFAULT_WINDOW_LOW`].
pub const DEFAULT_WINDOW_HIGH: f32 = 114.0;

/// Cells the widest window can be cut into, which is the table the boundary
/// angles are written to.
///
/// A cell boundary sits every 12 semitones out from half an octave either side
/// of the window's center, so the count is set by how many fit inside the
/// widest half-window there is — see
/// [`the_widest_window_fits_the_table`](self), which is what holds this to the
/// pitch limits rather than to a hope.
pub const MAX_CELLS: usize = 11;

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
/// picture — at the sharp end the octave beside the middle already keeps under
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

/// Shape the view starts at: the straight ramp, which is the middle of the
/// bar and the one shape that is neither a spotlight nor a plateau. It shows
/// nothing until the Amount leaves 0.
pub const DEFAULT_TAPER_SHAPE: f32 = 0.5;

/// A settable pair reduced to a window the layout can actually draw: inside
/// the pitch limits, low end first, and never narrower than [`MIN_WINDOW`].
///
/// The Range control cannot produce anything else — its bar carries the same
/// limit and the same minimum span — so this is for the pair that did not come
/// from it: a hand-edited blob, a project saved by a build whose limits were
/// different, a migration off the old octave count.
///
/// Widening happens at the HIGH end, and falls back to the low one only when
/// the ceiling is already reached: a too-narrow pair is nearly always one whose
/// high end was clamped or dropped, and growing upward keeps the low end — the
/// bass, which is where the music usually is — exactly where it was asked for.
pub fn clamp_window(low: f32, high: f32) -> (f32, f32) {
    let low = if low.is_finite() { low } else { DEFAULT_WINDOW_LOW };
    let high = if high.is_finite() { high } else { DEFAULT_WINDOW_HIGH };
    let (low, high) = if low <= high { (low, high) } else { (high, low) };
    let low = low.clamp(PITCH_FLOOR, PITCH_CEIL - MIN_WINDOW);
    let high = high.clamp(low + MIN_WINDOW, PITCH_CEIL);
    (low, high)
}

/// The pitch axis the octave indicators are drawn on, ready for the shader.
///
/// Computed once per frame from the view settings, not per node: every node
/// reads the SAME axis, which is what makes an indicator's angle mean an
/// absolute pitch rather than a position in that node's own private ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OctaveLayout {
    /// MIDI pitch at the seam approached from the LEFT — the lowest pitch the
    /// window shows. [`high_pitch`](Self::high_pitch) is the same point on the
    /// circle, come round the other way.
    pub low_pitch: f32,
    /// The highest pitch the window shows: the seam again, approached from the
    /// right.
    pub high_pitch: f32,
    /// Cells the window is cut into — whole octaves except at the two ends,
    /// where the window generally stops part way through one. The cells are
    /// where the taper's slope changes, and nothing else: an INDICATOR is a
    /// property of the node's pitch class (see [`Self::sector`]) and lines up
    /// with a cell only when the node shares the window's own pitch class.
    pub cells: u32,
    /// Pitch of the cell boundary the walk counts from, which is half an
    /// octave above the window's middle. Boundaries sit every 12 semitones
    /// from here, so they are symmetric about that middle — which is what puts
    /// the middle of the window at the top of the wheel under any taper.
    pub cell_origin: f32,
    /// Angle of each cell boundary, in radians, walking CLOCKWISE (the
    /// direction pitch rises) from the seam: `bounds[0]` is the bottom at
    /// `low_pitch`, and `bounds[cells]` is a full turn on — the same seam, at
    /// the window's high end. Entries past `cells` repeat the last so a stale
    /// index cannot produce a wild angle.
    pub bounds: [f32; MAX_CELLS + 1],
}

impl Default for OctaveLayout {
    fn default() -> Self {
        octave_layout(DEFAULT_WINDOW_LOW, DEFAULT_WINDOW_HIGH, 0.0, DEFAULT_TAPER_SHAPE)
    }
}

/// The pitch axis for the window `low..high`, tapered by `amount` in the shape
/// `shape`.
///
/// The seam is where the walk STARTS and a full turn is what it covers, which
/// is the whole of why the window's two ends meet at the bottom under any
/// settings: the invariant is structural rather than a property of the widths
/// that a new formula could break.
///
/// Two knobs that mean two different things, which is the whole reason the
/// taper is a pair of bars and not a list of named curves:
///
/// - The AMOUNT sets the EDGE cells, and nothing else does: they come out
///   `1 - amount` of an even slice at every shape and every window. That is a
///   size on screen rather than a ratio against another octave, which matters
///   because the octave a ratio would be against is the one the shape moves the
///   most — pinning the edge's RELATIVE weight instead let dragging Shape
///   toward the plateau, which widens everything but the middle, take degrees
///   away from the edge slices.
/// - The SHAPE says where the width the edges give up lands, through the
///   exponent `p` (see [`shape_exponent`]). Left of the bar's middle, `p` is
///   under 1 and the fall happens at once: the octave next to the middle gives
///   up most of what the edge one does and the outer ones flatten off, so the
///   middle keeps nearly all of the lift — a spotlight. Right of it `p` is over
///   1 and the fall is held back to the extremes: the octaves either side of
///   the middle stay nearly as wide as it is, a plateau rather than a gradient.
///
/// An even axis is `amount` 0, so it is where the bar starts rather than a mode
/// beside it — and the shape is inert there, which is exactly what "no taper"
/// should mean.
pub fn octave_layout(low: f32, high: f32, amount: f32, shape: f32) -> OctaveLayout {
    let (low_pitch, high_pitch) = clamp_window(low, high);
    let amount = amount.clamp(0.0, MAX_TAPER_AMOUNT);
    // Half an octave above the middle of the window. Cell boundaries sit every
    // octave from here, so the middle pitch is the CENTER of its own cell and
    // the cells either side of it mirror each other — the symmetry that puts
    // that pitch at the top of the wheel.
    let cell_origin = 0.5 * (low_pitch + high_pitch) + 0.5 * SEMIS;
    // The whole layout is computed in cells rather than semitones: `u` is the
    // pitch measured in octaves from `cell_origin`, so a cell boundary is an
    // integer and the two end cells are the only fractional lengths there are.
    let u_of = |pitch: f32| (pitch - cell_origin) / SEMIS;
    let (u_low, u_high) = (u_of(low_pitch), u_of(high_pitch));
    // The interior boundaries: every integer strictly inside the window. The
    // ends are the window's own, whole cell or part of one.
    let first = u_low.floor() + 1.0;
    let last = u_high.ceil() - 1.0;
    let interior = (last - first + 1.0).max(0.0) as usize;
    let cells = (interior + 1).min(MAX_CELLS);
    let mut edge = [0f32; MAX_CELLS + 1];
    edge[0] = u_low;
    for (j, e) in edge.iter_mut().enumerate().take(cells).skip(1) {
        *e = first + (j - 1) as f32;
    }
    edge[cells] = u_high;

    // How far each cell's middle is from the window's, normalized by the
    // furthest one — so the taper's shape is the same picture at every window
    // and only its resolution changes. The end cells are at distance 1, which
    // is what pins them to `1 - amount` whatever the shape does, and the cell
    // holding the window's middle is at 0.
    //
    // What the shape bends is how much each cell gives up on the way out,
    // `fall`, which is 0 at the middle and 1 at the edge whatever the exponent.
    let middle_u = u_of(0.5 * (low_pitch + high_pitch));
    let mid = |j: usize| 0.5 * (edge[j] + edge[j + 1]);
    let reach = (0..cells).fold(0f32, |far, j| far.max((mid(j) - middle_u).abs()));
    let p = shape_exponent(shape);
    let mut fall = [0f32; MAX_CELLS];
    let mut weighted = 0.0;
    for (j, f) in fall.iter_mut().enumerate().take(cells) {
        let dist = if reach > 0.0 { ((mid(j) - middle_u).abs() / reach).min(1.0) } else { 0.0 };
        *f = 1.0 - dist.powf(p);
        weighted += (edge[j + 1] - edge[j]) * *f;
    }

    // The widths, as an angle each. A cell's share is its LENGTH IN PITCH times
    // its density, so an even axis (amount 0) is one where every semitone is
    // worth the same angle however the cells fall — the end cells included,
    // which are the ones the window cut short. The lift is what the cells
    // inside the edge share out, and it is not a setting: the widths have to
    // add up to the circle, and that pins it.
    let span = u_high - u_low;
    let lift = span * amount / weighted.max(1e-6);
    let width = |j: usize| (edge[j + 1] - edge[j]) * ((1.0 - amount) + lift * fall[j]);
    let total = (0..cells).map(width).sum::<f32>().max(1e-6);
    // Clockwise is the direction pitch rises (uv.y is up, so the angle
    // decreases), which is why the walk subtracts.
    let mut bounds = [-FRAC_PI_2; MAX_CELLS + 1];
    let mut acc = 0.0;
    for j in 0..cells {
        acc += width(j);
        bounds[j + 1] = -FRAC_PI_2 - TAU * (acc / total);
    }
    for j in cells + 1..bounds.len() {
        bounds[j] = bounds[cells];
    }

    OctaveLayout { low_pitch, high_pitch, cells: cells as u32, cell_origin, bounds }
}

impl OctaveLayout {
    /// Semitones the window spans.
    pub fn span(&self) -> f32 {
        self.high_pitch - self.low_pitch
    }

    /// Where MIDI pitch `pitch` sits on the wheel, in radians. Linear within
    /// each cell and monotone across them, so an interval reads as an angle.
    ///
    /// Outside the window it CLAMPS, at either end. There is nothing out there
    /// to draw: an indicator that reaches past an end is cut off at the seam by
    /// [`sector`](Self::sector), which is what keeps the ring exactly covered
    /// when the window is not a whole number of octaves — continuing round
    /// instead would land at the wrong pitch, since one turn of a window like
    /// that does not come back to the same pitch class.
    pub fn angle(&self, pitch: f32) -> f32 {
        let (u_low, u_high) = (self.u_of(self.low_pitch), self.u_of(self.high_pitch));
        let u = self.u_of(pitch).clamp(u_low, u_high);
        // The same indexing the shader does, from the same three numbers: the
        // cell a pitch is in, and the fraction along it, with the two end cells
        // reaching only as far as the window does.
        let base = u_low.floor();
        let j = ((u.floor() - base).max(0.0) as usize).min(self.cells as usize - 1);
        let start = (base + j as f32).max(u_low);
        let end = (base + j as f32 + 1.0).min(u_high);
        let t = ((u - start) / (end - start).max(1e-6)).clamp(0.0, 1.0);
        self.bounds[j] + (self.bounds[j + 1] - self.bounds[j]) * t
    }

    /// Pitch in octaves from the cell origin — the units the boundary table is
    /// indexed in.
    fn u_of(&self, pitch: f32) -> f32 {
        (pitch - self.cell_origin) / SEMIS
    }

    /// MIDI pitch of octave slot `slot` on a node whose pitch class is
    /// `cents` (0..1200): the pitch that indicator stands for, and the pitch
    /// it is centered on.
    pub fn slot_pitch(&self, slot: u32, cents: f32) -> f32 {
        slot as f32 * SEMIS + cents / 100.0
    }

    /// The slots a node whose pitch class is `cents` draws, inclusive: the
    /// octaves of that pitch class whose own pitch lands inside the window.
    ///
    /// Per node, unlike the fixed span this replaced, and it has to be: the
    /// window is a pitch range rather than a count of octaves, so how many of a
    /// pitch class fall inside it depends on where that class sits. A window of
    /// nine and a half octaves holds nine of some classes and ten of others,
    /// and drawing the same NUMBERS on every node would then leave one node's
    /// ring with a bite out of it and another's overlapping itself.
    ///
    /// Notes outside it fold onto the nearest end (see `derive_scene`), so this
    /// is also what a voice's octave is clamped into.
    pub fn slot_range(&self, cents: f32) -> (u32, u32) {
        let offset = cents / 100.0;
        let last = OCTAVE_SLOTS as f32 - 1.0;
        let low = ((self.low_pitch - offset) / SEMIS).ceil().clamp(0.0, last);
        let high = ((self.high_pitch - offset) / SEMIS).floor().clamp(low, last);
        (low as u32, high as u32)
    }

    /// The two edge angles of slot `slot`'s indicator on a node whose pitch
    /// class is `cents` — the counter-clockwise one first. The shader's
    /// `oct_sector` is this.
    ///
    /// An indicator in the middle of the window is exactly its own octave, half
    /// of it either side of its own pitch. The two at the ENDS reach out to the
    /// seam instead, which is the whole of what makes a continuous window work:
    /// the window generally stops part way between two of a node's octaves, and
    /// the sliver left over belongs to the octave nearest it — the same octave
    /// a note out there folds onto. So the indicators still meet edge to edge
    /// and still close the ring, on every node, at any window.
    pub fn sector(&self, slot: u32, cents: f32) -> (f32, f32) {
        let (low_slot, high_slot) = self.slot_range(cents);
        let pitch = self.slot_pitch(slot, cents);
        let low = if slot <= low_slot { self.low_pitch } else { pitch - 0.5 * SEMIS };
        let high = if slot >= high_slot { self.high_pitch } else { pitch + 0.5 * SEMIS };
        (self.angle(low), self.angle(high))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    /// The Shape bar at both ends, at its middle, and either side of it: the
    /// sharpest spotlight, the straight ramp, and the flattest plateau.
    const SHAPES: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

    /// Windows to run the invariants over: the default, the narrowest and the
    /// widest, and four that are deliberately NOT a whole number of octaves or
    /// not centered on a C — which is the whole point of the range being
    /// continuous, and the case every "it tiles" argument has to survive.
    const WINDOWS: [(f32, f32); 8] = [
        (DEFAULT_WINDOW_LOW, DEFAULT_WINDOW_HIGH),
        (PITCH_FLOOR, PITCH_CEIL),
        // The narrowest there is, which is C1..C5 in the DAW's numbering.
        (36.0, 84.0),
        (36.0, 96.0),
        (36.0, 97.5),
        (30.5, 100.25),
        (0.0, 60.0),
        (61.0, 127.0),
    ];

    /// Every window, shape and amount, against pitch classes that put an
    /// indicator's edge exactly on a cell boundary (C), well clear of it, and
    /// just short of it — the grid the invariants below all run over.
    fn every_case() -> impl Iterator<Item = (OctaveLayout, f32, String)> {
        WINDOWS.iter().flat_map(|&(low, high)| {
            SHAPES.iter().flat_map(move |&shape| {
                [0.0f32, 0.35, 0.9].iter().flat_map(move |&amount| {
                    [0.0f32, 350.0, 700.0, 1150.0].iter().map(move |&cents| {
                        (
                            octave_layout(low, high, amount, shape),
                            cents,
                            format!("{low}..{high}, amount {amount} shape {shape}, {cents}c"),
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
            let high = l.angle(l.high_pitch);
            assert!((high - (-FRAC_PI_2 - TAU)).abs() < 1e-4, "{case}: {high}");
        }
    }

    /// The middle of the window is straight up — on the axis itself, so on
    /// every node. With the default window, which is centered on middle C, that
    /// is middle C, which is what makes the wheel read like a keyboard.
    #[test]
    fn the_middle_of_the_window_is_straight_up() {
        for (l, _, case) in every_case() {
            let up = l.angle(0.5 * (l.low_pitch + l.high_pitch));
            assert!((up - (-FRAC_PI_2 - PI)).abs() < 1e-4, "{case}: {up}");
        }
        let default = OctaveLayout::default();
        assert!((default.angle(60.0) - (-FRAC_PI_2 - PI)).abs() < 1e-4, "middle C is up");
    }

    /// What "faithful" means, as an assertion: the map is strictly falling
    /// in pitch, and equal intervals WITHIN one cell subtend equal angles,
    /// so an indicator sits on its pitch rather than near it.
    #[test]
    fn the_axis_is_monotone_and_linear_inside_a_cell() {
        for (l, _, case) in every_case() {
            let mut previous = l.angle(l.low_pitch);
            let mut pitch = l.low_pitch;
            while pitch < l.high_pitch - 0.25 {
                pitch += 0.5;
                let a = l.angle(pitch);
                assert!(a < previous, "{case}: not falling at {pitch}");
                previous = a;
            }
            // Three points evenly spaced in pitch inside the cell holding the
            // window's middle come out evenly spaced in angle.
            let base = 0.5 * (l.low_pitch + l.high_pitch) - 4.0;
            let (a, b, c) = (l.angle(base), l.angle(base + 4.0), l.angle(base + 8.0));
            assert!(((a - b) - (b - c)).abs() < 1e-4, "{case}: uneven inside a cell");
        }
    }

    /// A drawn indicator is on its own pitch, and in the middle of the window
    /// it is exactly one octave — cut short at the seam only where the window
    /// itself stops. That is what keeps the mapping honest: an indicator moved
    /// to fit would misstate its pitch.
    #[test]
    fn drawn_indicators_sit_on_their_own_pitch() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range(cents);
            assert!(low <= high, "{case}: nothing drawn");
            for slot in low..=high {
                let pitch = l.slot_pitch(slot, cents);
                assert!(
                    pitch >= l.low_pitch - 1e-3 && pitch <= l.high_pitch + 1e-3,
                    "{case}: slot {slot} is drawn at {pitch}, outside the window"
                );
                let (e0, e1) = l.sector(slot, cents);
                assert!(e0 > e1, "{case}: slot {slot} runs backwards");
                let inside = l.angle(pitch);
                assert!(e0 >= inside && inside >= e1, "{case}: slot {slot} misses its own pitch");
                // An interior indicator is its own octave and nothing else.
                if slot > low && slot < high {
                    assert!(
                        (e0 - l.angle(pitch - 6.0)).abs() < 1e-4
                            && (e1 - l.angle(pitch + 6.0)).abs() < 1e-4,
                        "{case}: slot {slot} is not a whole octave"
                    );
                }
            }
        }
    }

    /// The indicators tile the turn: each one's clockwise edge is the next
    /// one's counter-clockwise edge, the lowest starts at the seam and the
    /// highest ends there. So the ring closes on every node whatever its pitch
    /// class and whatever the window — which a wheel divided into whole octaves
    /// gets for free and a continuous one has to be built for.
    #[test]
    fn the_indicators_tile_the_turn() {
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range(cents);
            let mut total = 0.0;
            for slot in low..=high {
                let (e0, e1) = l.sector(slot, cents);
                total += e0 - e1;
                if slot < high {
                    let next = l.sector(slot + 1, cents).0;
                    assert!((e1 - next).abs() < 1e-4, "{case}: slot {slot} leaves {}", e1 - next);
                }
            }
            assert!((total - TAU).abs() < 1e-4, "{case}: the ring covers {total} rad");
        }
    }

    /// Octaves shrink outward, symmetrically, at every shape — and only when
    /// an amount is asked for: the middle of the window is what carries the
    /// extra weight.
    #[test]
    fn octaves_shrink_away_from_the_middle() {
        for shape in SHAPES {
            for amount in [0.0f32, 0.6] {
                // Nine octaves centered on middle C — an odd count on a C is
                // what puts both ends on a cell boundary, so every cell is a
                // full octave and the widths compare directly.
                let l = octave_layout(6.0, 114.0, amount, shape);
                let width = |j: usize| l.bounds[j] - l.bounds[j + 1];
                let cells = l.cells as usize;
                for j in 0..cells / 2 {
                    let (inner, outer) = (width(j + 1), width(j));
                    let case = format!("shape {shape}, amount {amount}, cell {j}");
                    if amount == 0.0 {
                        assert!((inner - outer).abs() < 1e-5, "{case}: an even axis moved");
                    } else {
                        assert!(outer < inner, "{case}: {outer} !< {inner}");
                    }
                    // Symmetric about the middle cell.
                    assert!(
                        (width(j) - width(cells - 1 - j)).abs() < 1e-5,
                        "{case}: lopsided"
                    );
                }
            }
        }
    }

    /// The Amount alone sets the edge slices, and the Shape leaves them
    /// exactly where they are: an edge octave is `1 - amount` of an even
    /// slice at every shape and every window. Dragging Shape across its whole
    /// travel must not cost the outermost octaves a degree — they are the
    /// ones with the least to give.
    ///
    /// Windows made of whole cells, because those are the ones where "an even
    /// slice" is a number rather than a figure of speech: the end cells of any
    /// other window are short, and a short cell is legitimately worth less than
    /// a full one. That means an ODD number of octaves centered on a C — the
    /// count puts the window's ends half an octave out from the outermost C,
    /// which is exactly where the cell boundaries fall.
    #[test]
    fn the_amount_alone_sets_the_edge_slices() {
        for octaves in [5u32, 7, 9] {
            let span = octaves as f32 * 12.0;
            let (low, high) = (60.0 - span * 0.5, 60.0 + span * 0.5);
            for amount in [0.0f32, 0.35, MAX_TAPER_AMOUNT] {
                let even = TAU / octaves as f32;
                for shape in SHAPES {
                    let l = octave_layout(low, high, amount, shape);
                    assert_eq!(l.cells, octaves, "{octaves} octaves came out {} cells", l.cells);
                    let edge = l.bounds[0] - l.bounds[1];
                    let case = format!("{octaves} octaves, amount {amount}, shape {shape}");
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
    /// between the middle octave and the ones out toward them. Every step
    /// right flattens the profile — the middle narrows and the octaves next
    /// to the edges widen — and every step left concentrates it again.
    #[test]
    fn the_shape_flattens_the_profile_between_the_edges() {
        let width = |shape: f32, j: usize| {
            let l = octave_layout(6.0, 114.0, 0.6, shape);
            (l.bounds[j] - l.bounds[j + 1]).to_degrees()
        };
        // Nine octaves: the middle one is index 4, and index 1 is the widest
        // one the shape can still reach, just inside the pinned edge.
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
    /// anything. The widest there is is the middle indicator at the narrowest
    /// window, the fullest amount and the sharpest shape — five octaves to the
    /// turn, the edges pinned at a tenth of an even slice and the middle taking
    /// everything they and their neighbours give up.
    ///
    /// This is what [`MIN_WINDOW`] exists for: the same settings over a
    /// three-octave window take the middle indicator to 336 degrees, and a
    /// two-octave one to 342.
    #[test]
    fn an_indicator_can_pass_a_half_turn_but_never_a_whole_one() {
        let mut widest: f32 = 0.0;
        for (l, cents, case) in every_case() {
            let (low, high) = l.slot_range(cents);
            for slot in low..=high {
                let (e0, e1) = l.sector(slot, cents);
                assert!(e0 - e1 < TAU - 1e-3, "{case}: slot {slot} spans {} rad", e0 - e1);
                widest = widest.max(e0 - e1);
            }
        }
        assert!(widest > PI, "nothing passes a half turn: widest {widest} rad");
        // Sweeping the NARROWEST window along the axis at every pitch class,
        // since the widest indicator there is is the one the floor is set to
        // keep in hand: where the window sits, and where the node's octaves
        // fall inside it, decide how much of the turn one of them can hold.
        //
        // The threshold is a good deal tighter than a whole turn, because a
        // measurement 15 degrees off the cliff is not a guard. At MIN_WINDOW
        // this reaches 266 degrees; three and a half octaves would put it at
        // 274 and three octaves at 336.
        let mut narrowest: f32 = 0.0;
        for low in 0..=(PITCH_CEIL - MIN_WINDOW) as u32 {
            for cents in [0.0f32, 100.0, 350.0, 600.0, 700.0, 1150.0] {
                let l = octave_layout(low as f32, low as f32 + MIN_WINDOW, MAX_TAPER_AMOUNT, 0.0);
                let (a, b) = l.slot_range(cents);
                for slot in a..=b {
                    let (e0, e1) = l.sector(slot, cents);
                    narrowest = narrowest.max(e0 - e1);
                }
            }
        }
        assert!(
            narrowest < TAU * 0.78,
            "the narrowest window reaches {} deg",
            narrowest.to_degrees()
        );
    }

    /// A window outside the settable pair — a hand-edited blob, a migration —
    /// is brought back inside it rather than producing a layout the shader's
    /// fixed-size tables cannot hold.
    #[test]
    fn a_window_outside_the_limits_is_clamped() {
        assert_eq!(clamp_window(60.0, 60.0), (60.0, 60.0 + MIN_WINDOW), "a collapsed window opens");
        assert_eq!(clamp_window(114.0, 6.0), (6.0, 114.0), "a backwards pair is read either way");
        assert_eq!(clamp_window(-40.0, 400.0), (PITCH_FLOOR, PITCH_CEIL), "and the limits hold");
        let (low, high) = clamp_window(120.0, 121.0);
        assert!(high - low >= MIN_WINDOW && high <= PITCH_CEIL, "{low}..{high} at the ceiling");
        let nan = clamp_window(f32::NAN, f32::INFINITY);
        assert_eq!(nan, (DEFAULT_WINDOW_LOW, DEFAULT_WINDOW_HIGH), "nonsense falls back");
    }

    /// The widest window still fits the boundary table, which the renderer's
    /// uniform is sized against. Swept rather than reasoned, because the count
    /// depends on where the window's middle falls between two cell boundaries
    /// as well as on how wide it is.
    #[test]
    fn the_widest_window_fits_the_table() {
        let mut most = 0;
        let mut at = (0.0, 0.0);
        for low in 0..=127 {
            for high in low..=127 {
                let (low, high) = (low as f32, high as f32);
                if high - low < MIN_WINDOW {
                    continue;
                }
                for offset in [0.0f32, 0.25, 0.5, 0.75] {
                    let l = octave_layout(low, high + offset, 0.0, DEFAULT_TAPER_SHAPE);
                    if l.cells as usize > most {
                        most = l.cells as usize;
                        at = (l.low_pitch, l.high_pitch);
                    }
                }
            }
        }
        assert!(most <= MAX_CELLS, "{at:?} needs {most} cells, and the table holds {MAX_CELLS}");
        assert_eq!(
            most, MAX_CELLS,
            "the table is {} cells bigger than anything needs",
            MAX_CELLS - most
        );
    }

    /// A whole number of octaves centered on middle C is the wheel the fixed
    /// span used to draw, cell for cell — so the continuous window is a
    /// generalization of it rather than a different picture that happens to
    /// look similar.
    #[test]
    fn a_whole_number_of_octaves_draws_the_span_it_replaced() {
        // ±2 to ±4. The old ±5 reached MIDI -6, half an octave under the
        // lowest note there is; it migrates to a window that starts at 0
        // instead, which draws the same eleven octaves with the bottom one
        // cut off at the seam rather than centered on it.
        for span in 2..=4u32 {
            let octaves = 2 * span + 1;
            let low = 60.0 - 6.0 * octaves as f32;
            let l = octave_layout(low, low + 12.0 * octaves as f32, 0.35, 0.25);
            assert_eq!(l.cells, octaves, "±{span} is {octaves} cells");
            let middle = MIDDLE_C_SLOT as u32;
            assert_eq!(l.slot_range(0.0), (middle - span, middle + span));
            // Every cell a whole octave, tiling the turn evenly about the
            // middle, and middle C dead center of its own.
            for j in 0..octaves as usize {
                let width = l.bounds[j] - l.bounds[j + 1];
                let mirror = l.bounds[octaves as usize - 1 - j] - l.bounds[octaves as usize - j];
                assert!((width - mirror).abs() < 1e-5, "±{span}: cell {j} is lopsided");
            }
            let (e0, e1) = l.sector(MIDDLE_C_SLOT as u32, 0.0);
            let middle_c = l.angle(60.0);
            assert!((middle_c - 0.5 * (e0 + e1)).abs() < 1e-4, "±{span}: middle C off center");
        }
    }
}
