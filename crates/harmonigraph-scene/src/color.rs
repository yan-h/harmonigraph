//! Node color: the pitch ramp shared by the CPU and the shader LUT, plus
//! channel and idle colors.
//!
//! The ramp is authored in two spaces at once, which is the whole design:
//! its LIGHTNESS is CIELAB's `L*` and its HUE and CHROMA are Oklab's. `L*`
//! is a function of luminance alone, so one `L*` is one screen brightness
//! exactly and a flat ramp is flat to the pixel; Oklab's hue is the one that
//! holds still when only the chroma knob moves, where a CIELAB hue angle
//! drifts up to 19 degrees toward purple through the blues the default arc
//! opens on. Neither space delivers both.
//!
//! Google's HCT is the same structure over CAM16 rather than Oklab, for a
//! different reason (predictable WCAG contrast between tones). Oklab is the
//! cheaper half of that trade and the whole of it that matters here — see
//! `tests/hue_space.rs`, which measures both and prices them.
//!
//! The fixed per-channel colors below stay hand-written CIELAB LCh. They name
//! voices rather than pitches, nothing about them is swept, and re-authoring
//! them in another space would move colors for no gain.

use crate::view::ViewConfig;
use crate::style::PitchGradient;
use harmonigraph_core::ChannelRole;
use crate::PITCH_LUT_N;
use glam::Vec4;

fn lch(l: f64, c: f64, h: f64) -> Vec4 {
    // The conversion is unclamped and out-of-gamut LCH inputs yield values
    // outside 0..255 (v1's graphics stack clamped downstream; we must do it
    // ourselves before handing colors to the shader).
    //
    // Only the fixed channel colors come through here, and they are
    // hand-written LCh — this is what keeps a mistyped one drawable. The pitch
    // gradient has its own path and is inside the gamut by construction.
    let rgb = color_space::Rgb::from(color_space::Lch::new(l, c, h));
    Vec4::new(
        (rgb.r.clamp(0.0, 255.0) / 255.0) as f32,
        (rgb.g.clamp(0.0, 255.0) / 255.0) as f32,
        (rgb.b.clamp(0.0, 255.0) / 255.0) as f32,
        1.0,
    )
}

/// Normalized pitch height in 0..1 across the gradient range: 0 at
/// `darkest_pitch`, 1 at `brightest_pitch` (both MIDI note numbers).
///
/// The RATIO is what gets clamped, which is the shader's own form
/// (`pitch_lut_color` in `lattice.wgsl`) and the reason anything can be
/// colored to match what the shader draws. Clamping the PITCH into
/// `darkest..brightest` instead reads the same over every range the Nodes
/// pane can dial, and comes apart at the two the pane cannot: the ends are
/// independent params over 0..120, ordered only by the range bar's min span,
/// so a host reaches an inverted pair — where `f32::clamp` panics on
/// `min > max` — and a collapsed one, where clamping the pitch first pins
/// every note to the dark end while the shader takes the bright one.
///
/// Non-finite ends yield the darkest color rather than a NaN that would ride
/// into the instance buffer unnoticed.
fn pitch_ramp_t(pitch: f32, darkest_pitch: f32, brightest_pitch: f32) -> f64 {
    if !darkest_pitch.is_finite() || !brightest_pitch.is_finite() {
        return 0.0;
    }
    f64::from(
        ((pitch - darkest_pitch) / (brightest_pitch - darkest_pitch).max(0.01)).clamp(0.0, 1.0),
    )
}

/// The luminance one `L*` names, CIE's own piecewise inverse.
///
/// This function is the entire reason the ramp can mix two spaces: `L*`
/// depends on Y and nothing else, so a lightness stated in `L*` survives being
/// carried into a space that has its own idea of lightness. Oklab's `L` does
/// not have this property — it is a nonlinear combination of cube-rooted LMS,
/// and a constant Oklab `L` is not a constant luminance.
fn luminance_of_lightness(l_star: f64) -> f64 {
    let fy = (l_star + 16.0) / 116.0;
    if fy > 6.0 / 29.0 {
        fy.powi(3)
    } else {
        3.0 * (6.0 / 29.0f64).powi(2) * (fy - 4.0 / 29.0)
    }
}

/// The `L*` an encoded sRGB triple carries — [`luminance_of_lightness`] run
/// backwards, with the transfer function undone first.
///
/// Public because reading a rendered pixel back is the only way to check a
/// claim about how bright the picture LOOKS, and that reading has to be in the
/// units the ramp is authored in or it answers a different question: a channel
/// sum weights blue like green and calls a violet and a yellow of the same sum
/// equally bright, which they are not. Sharing the curve rather than restating
/// it is the point — a copy that drifted from these constants would agree with
/// itself and disagree with the picture.
pub fn lightness_of_encoded(r: f64, g: f64, b: f64) -> f64 {
    let y = 0.2126 * linear_of_encoded(r)
        + 0.7152 * linear_of_encoded(g)
        + 0.0722 * linear_of_encoded(b);
    if y > (6.0 / 29.0f64).powi(3) {
        116.0 * y.cbrt() - 16.0
    } else {
        y * (29.0 / 3.0f64).powi(3)
    }
}

/// Linear sRGB from Oklab, by Ottosson's matrices.
///
/// Linear and not encoded, deliberately: the gamut test below runs on this,
/// and the 0..1 box is the same box either side of the transfer function, so
/// testing here skips a gamma encode on every bisection step. That argument
/// carries for the BOX and not for a tolerance around it — see [`gamut_box`].
fn linear_srgb_of_oklab(ok_l: f64, ok_a: f64, ok_b: f64) -> (f64, f64, f64) {
    let l = (ok_l + 0.3963377774 * ok_a + 0.2158037573 * ok_b).powi(3);
    let m = (ok_l - 0.1055613458 * ok_a - 0.0638541728 * ok_b).powi(3);
    let s = (ok_l - 0.0894841775 * ok_a - 1.2914855480 * ok_b).powi(3);
    (
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    )
}

/// sRGB's own luminance row, folded through the LMS->linear-RGB matrix: the
/// weight each CUBED Oklab root carries in the luminance of the color.
///
/// Folded rather than applied in two steps so the Newton solve below never
/// builds an RGB triple it would only collapse again. What matters about the
/// numbers is their SIGNS: the first and third are negative (-0.0408 and
/// -0.0717) against a positive 1.1125 in the middle, which is the whole reason
/// luminance is not monotone in `L` and the solve needs a guard.
const LUMINANCE_OF_ROOTS: [f64; 3] = [
    0.2126 * 4.0767416621 + 0.7152 * -1.2684380046 + 0.0722 * -0.0041960863,
    0.2126 * -3.3077115913 + 0.7152 * 2.6097574011 + 0.0722 * -0.7034186147,
    0.2126 * 0.2309699292 + 0.7152 * -0.3413193965 + 0.0722 * 1.7076147010,
];

/// Newton steps [`RampHue::lightness_at`] spends. Five, because the function it
/// inverts is a cubic and the initial guess is already close: over `L*` 0..100
/// x 360 hues x five chroma fractions, every solve the gamut search grants
/// lands within a relative 2.2e-15 of the luminance it was asked for, which is
/// the double's own resolution there and a dozen orders under the quantization
/// the answer is heading for.
///
/// A coverage knob and NOT a correctness one, which is the whole point of
/// [`LUMINANCE_MISS`] sitting beside it: a solve that fails to land is read as
/// "no color here" by [`RampHue::in_gamut`] rather than trusted, so this
/// changes how much of the gamut the search can reach and cannot move a drawn
/// color off its luminance. Measured across the same sweep, 4, 5, 6, 8 and 20
/// steps all land at that same resolution and grant the identical gamut; at 3
/// the worst granted solve sits exactly ON the tolerance, with the guard the
/// only thing still holding. One step of margin over that is what five is, and
/// `the_hybrid_at_the_edges` is where both ends were measured.
const LUMINANCE_STEPS: u32 = 5;

/// How far a solve may land from the luminance it was asked for and still count
/// as having found it, as a FRACTION of that luminance.
///
/// The distinction this draws is convergence, not rounding — which is why it
/// can afford to sit six orders above the worst converged miss. Newton on a
/// cubic from a start this good either lands at the double's own resolution or
/// does not land at all, and a solve that did not land is not a near miss but
/// an unrelated color, whose channels are perfectly capable of sitting inside
/// the box. That is the failure it exists to catch: the search reads such an
/// `L` as a drawable color and grants chroma the ramp then paints, which at
/// `L*` 0 puts a bright pixel where the curve asked for black.
///
/// Relative and not absolute, because the luminance it is a tolerance ON spans
/// the whole 0..1 range and reaches 0 exactly. Any absolute figure is nothing
/// but noise at the black end — every color is within 1e-9 of luminance 0,
/// diverged ones included, so an absolute test stops discriminating exactly
/// where the failure lives. At `L*` 0 this asks for luminance 0 exactly, which
/// is the right question: black is the only color at that luminance, and a
/// solve that reached any other has not found it.
const LUMINANCE_MISS: f64 = 1e-9;

/// One hue, with everything a gamut search at that hue needs worked out once.
///
/// A struct because [`max_chroma`] asks about twenty chromas at a FIXED hue and
/// a fixed luminance: the trig pair and the three offsets are invariant across
/// all twenty, and re-deriving them per step is the difference between a free
/// LUT rebuild and a visible one on every frame of a Brightness or Chroma drag.
#[derive(Clone, Copy)]
struct RampHue {
    cos_h: f64,
    sin_h: f64,
    /// How far each LMS cube root runs from `L` at this hue, per unit of
    /// chroma. Oklab's `a` and `b` enter [`linear_srgb_of_oklab`] linearly, so
    /// a fixed hue makes each root an affine function of `L` and `c` alone.
    k: [f64; 3],
}

impl RampHue {
    fn at(h: f64) -> RampHue {
        let (sin_h, cos_h) = h.to_radians().sin_cos();
        RampHue {
            cos_h,
            sin_h,
            k: [
                0.3963377774 * cos_h + 0.2158037573 * sin_h,
                -0.1055613458 * cos_h - 0.0638541728 * sin_h,
                -0.0894841775 * cos_h - 1.2914855480 * sin_h,
            ],
        }
    }

    /// The Oklab `L` that puts this hue and chroma at `target_y` — the join
    /// between the two spaces, and where `L*`'s promise is actually kept.
    ///
    /// Newton and not a second bisection, which is what makes the whole design
    /// affordable. At a FIXED hue the three cube roots are each linear in `L`,
    /// so luminance is a cubic in `L` with a derivative in closed form; a
    /// nested bisection would spend twenty steps where this spends five, and
    /// turn a rebuild from free into a visible hitch on every frame of a knob
    /// drag.
    ///
    /// May return an `L` that does NOT reach `target_y`, and says so only by
    /// the luminance of what it hands back — see [`in_gamut`](Self::in_gamut),
    /// which is where that answer is read. Every ask reaching the draw path has
    /// been through the gamut search first and does land.
    fn lightness_at(&self, c: f64, target_y: f64) -> f64 {
        let w = &LUMINANCE_OF_ROOTS;
        // The grey of this luminance, which is exact at chroma 0 and near it
        // everywhere else — Oklab `L` IS the cube root of luminance on the
        // neutral axis.
        let mut ok_l = target_y.cbrt().clamp(0.0, 1.0);
        for _ in 0..LUMINANCE_STEPS {
            let r = [ok_l + self.k[0] * c, ok_l + self.k[1] * c, ok_l + self.k[2] * c];
            let y: f64 = (0..3).map(|i| w[i] * r[i].powi(3)).sum();
            let dy: f64 = (0..3).map(|i| 3.0 * w[i] * r[i].powi(2)).sum();
            // Only a RISING luminance carries the step toward the answer, and
            // this one does not always rise: two of the three weights are
            // negative, so the derivative goes negative wherever the middle
            // root is small beside the other two — which is where the bottom of
            // the `L*` axis meets a saturated hue. A step taken there walks away
            // from the root and the clamp below parks it at a bright color, so
            // stopping is the only move that keeps the miss legible.
            //
            // At chroma 0 the same guard catches the one case where stopping IS
            // the answer: black, where `L` is 0, the luminance asked for has
            // already been reached, and a step would divide by nothing.
            if dy <= 1e-12 {
                break;
            }
            // Clamped past 1 rather than to it: an overshoot on the way to a
            // bright color is a legal intermediate, and pinning it at 1 would
            // stall the iteration instead of letting it come back.
            ok_l = (ok_l - (y - target_y) / dy).clamp(0.0, 1.5);
        }
        ok_l
    }

    /// Linear sRGB of one point of the ramp: solve for the `L` that hits
    /// `target_y`, then convert. The two questions asked of a ramp color —
    /// is it drawable, and what bytes is it — differ only in what they do with
    /// this, so the solve and the conversion live here once rather than twice.
    fn linear_srgb(&self, c: f64, target_y: f64) -> (f64, f64, f64) {
        let ok_l = self.lightness_at(c, target_y);
        linear_srgb_of_oklab(ok_l, c * self.cos_h, c * self.sin_h)
    }

    /// Whether this hue at this chroma is a color sRGB can show at `target_y`,
    /// with each linear channel allowed anywhere inside `box_` (see
    /// [`gamut_box`]).
    ///
    /// Two conditions and not one. The channels have to be in the box, and the
    /// solve has to have LANDED on the luminance that was asked for — a
    /// diverged `L` is an unrelated color rather than a near miss, and its
    /// channels sit comfortably inside the box for a band of hues at the black
    /// end of the `L*` axis. Read off the same linear RGB the box test runs on,
    /// so what is checked is the luminance of the color that would actually be
    /// drawn rather than of the coordinates it came from.
    fn in_gamut(&self, c: f64, target_y: f64, box_: (f64, f64)) -> bool {
        let (r, g, b) = self.linear_srgb(c, target_y);
        let reached = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let inside = |v: f64| (box_.0..=box_.1).contains(&v);
        (reached - target_y).abs() <= LUMINANCE_MISS * target_y
            && inside(r)
            && inside(g)
            && inside(b)
    }
}

/// The sRGB transfer function's inverse: the linear light an encoded value
/// names. Extends past 1 by simply being evaluated there, which is what lets
/// [`gamut_box`] ask how far outside the box half a byte reaches.
fn linear_of_encoded(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// The linear-light box a channel must land in, widened by `slack` steps of
/// 1/255 past either end.
///
/// The slack is stated in ENCODED steps because that is the only place it means
/// anything: the answer is quantized to a byte, so "less than half a step"
/// is a claim about bytes. The SAME figure in linear light is not one tolerance
/// but two — half a byte spans 1.5e-4 of linear light at the black end and
/// 4.5e-3 at the white end, a factor of 29 — so any single linear slack is
/// generous at one end and absent at the other. Converting once here, per
/// search rather than per bisection step, keeps the meaning and the cost.
///
/// A zero slack costs nothing and changes nothing: the transfer function fixes
/// both 0 and 1, so the box comes back exactly 0..1 and [`max_chroma`]'s strict
/// predicate is the same predicate it was in linear light.
fn gamut_box(slack: f64) -> (f64, f64) {
    (-linear_of_encoded(slack / 255.0), linear_of_encoded(1.0 + slack / 255.0))
}

/// Whether the color this ramp asks for is one sRGB can show, allowing each
/// channel `slack` steps of 1/255 past either end of the box.
///
/// The whole-coordinates form of [`RampHue::in_gamut`], which is what
/// [`max_chroma`] wants instead: a search at a fixed hue and lightness derives
/// all three of these once and asks about twenty chromas.
#[cfg(test)]
fn in_gamut_within(l_star: f64, h: f64, c: f64, slack: f64) -> bool {
    RampHue::at(h).in_gamut(c, luminance_of_lightness(l_star), gamut_box(slack))
}

/// Bisection steps [`max_chroma`] spends. Each halves the bracket, so 20 over
/// `0..MAX_SEARCH_CHROMA` settles to well under a millionth of a chroma unit —
/// far finer than the 1/255 the answer is eventually quantized to, and the
/// whole search runs only when the gradient changes.
const GAMUT_BISECTIONS: u32 = 20;

/// Chroma the search brackets from above. Oklab's most saturated sRGB color
/// reaches about 0.32, so nothing is ever cut off by this; it is the bracket
/// rather than a limit.
const MAX_SEARCH_CHROMA: f64 = 0.4;

/// The largest Oklab chroma sRGB can show at this `L*` and hue.
///
/// Bisection, which needs the in-gamut chromas at a fixed lightness and hue to
/// be one interval running out from the neutral axis. They are: the gamut is a
/// convex solid in linear RGB, and holding luminance fixed cuts it with a
/// plane, leaving a convex section that the neutral of that luminance sits
/// inside — so every ray out from that neutral crosses the boundary once.
///
/// Bisection is what makes the chroma knob possible at all: the answer varies
/// with BOTH lightness and hue, over a boundary with no closed form, so a
/// gradient free to move either one cannot carry a chroma figure that was
/// worked out in advance.
fn max_chroma(l_star: f64, h: f64) -> f64 {
    let hue = RampHue::at(h);
    let target_y = luminance_of_lightness(l_star);
    let box_ = gamut_box(0.0);
    let (mut lo, mut hi) = (0.0f64, MAX_SEARCH_CHROMA);
    for _ in 0..GAMUT_BISECTIONS {
        let mid = 0.5 * (lo + hi);
        if hue.in_gamut(mid, target_y, box_) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    // `lo` and never `hi`: the low side of the bracket is the one the test has
    // actually accepted, so what comes back is in gamut rather than within a
    // bisection step of it.
    lo
}

/// One color of the ramp: `L*` for lightness, an Oklab hue, and an ABSOLUTE
/// Oklab chroma (the fraction is resolved by [`pitch_ramp_coords`]).
///
/// The clamp is a safety net over a path that should not need it — every
/// chroma reaching here is under what [`max_chroma`] granted, and the search
/// grants only chromas whose solve lands — and it is what keeps a color
/// drawable if the Newton solve is ever handed a target it cannot reach.
fn oklab_srgb(l_star: f64, h: f64, c: f64) -> Vec4 {
    let (r, g, b) = RampHue::at(h).linear_srgb(c, luminance_of_lightness(l_star));
    let encode = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        (if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }) as f32
    };
    Vec4::new(encode(r), encode(g), encode(b), 1.0)
}

/// The DESIGNED pitch-gradient curve as a function of normalized height `t`
/// (0..1) and the gradient in force. Nothing draws this directly: it is the
/// curve [`pitch_ramp_lut`] samples, and that table is what every consumer
/// reads (see [`pitch_lut_color`]). Keeping one caller means the table and
/// the curve can never drift into two different answers for one pitch.
///
/// Chroma is the only one of the four knobs that is not simply read off
/// [`PitchGradient`]: it arrives as a FRACTION of what the gamut holds here,
/// so the curve stays inside sRGB at every setting of the other three and
/// `L*` — hence the luminance — is exactly what was asked for. See
/// [`PitchGradient::chroma`] for why an absolute chroma cannot be, and
/// `the_gradient_is_in_gamut_and_flat_when_its_ramp_is` for what holds this to
/// it.
fn pitch_ramp_color(t: f64, gradient: PitchGradient) -> Vec4 {
    let (l, h, c) = pitch_ramp_coords(t, gradient);
    oklab_srgb(l, h, c)
}

/// The three coordinates [`pitch_ramp_color`] converts, before the conversion:
/// `L*`, an Oklab hue, and an absolute Oklab chroma.
///
/// Split out for [`ramp_sample_in_gamut`], which has to ask about the color
/// that was REQUESTED: [`oklab_srgb`] clamps, so a color read back out of the
/// table is inside sRGB whatever was asked for, and a clamp cannot also be the
/// check on whether it was needed.
///
/// Takes an already-sanitized gradient and does not re-sanitize. The invariant
/// has boundaries rather than a scattering of owners — [`with_lut`] applies it
/// once for a whole table (and keys the memo on the result),
/// [`designed_pitch_ramp`] does it for the test path, and
/// [`PitchGradient::lightness_and_hue`] holds up its own public end. Only
/// `chroma` is read raw here, which is what lets the gamut test hand in the
/// out-of-range fraction a control cannot produce.
fn pitch_ramp_coords(t: f64, gradient: PitchGradient) -> (f64, f64, f64) {
    let (l, h) = gradient.lightness_and_hue(t);
    (l, h, f64::from(gradient.chroma) * max_chroma(l, h))
}

/// Whether the curve's ask at `t` is a color sRGB can actually show, put to the
/// strict predicate instead of to the clamped result. For
/// `the_gradient_is_in_gamut_and_flat_when_its_ramp_is`, which cannot learn
/// anything by reading the table back: every entry there has already been
/// clamped into range by [`oklab_srgb`].
///
/// Deliberately NOT sanitizing, so the test can hand in a chroma past 1.0 and
/// watch this say no — a check whose failing case is unreachable is not a
/// check.
///
/// Half a quantization step of slack, where [`max_chroma`]'s own predicate
/// takes none. The strictness there is what keeps the clamp idle: a bisection
/// that stopped a hair OUTSIDE would hand back a chroma [`oklab_srgb`] then has
/// to clip. Asked the other question — was the clamp needed? — that same
/// strictness reads the ends of the `L*` axis as out of gamut, since the
/// Newton solve lands a whisker either side of exactly white or exactly black
/// and the sweep reaches both wherever a steep ramp flattens against the top or
/// bottom. A clamp that moves a channel by less than half a byte moves no byte
/// at all, and [`gamut_box`] is where that half-byte keeps its meaning at both
/// ends rather than at one.
///
/// The luminance half of the predicate takes no slack in either caller, and
/// wants none: it separates a solve that converged from one that did not, which
/// is not a question of a hair either way.
#[cfg(test)]
pub(crate) fn ramp_sample_in_gamut(t: f64, gradient: PitchGradient) -> bool {
    let (l, h, c) = pitch_ramp_coords(t, gradient);
    in_gamut_within(l, h, c, 0.5)
}

/// The designed curve, for the test that pins how closely [`pitch_ramp_lut`]
/// tracks it. Nothing in the draw path may call this — going off the curve
/// direct is precisely the mismatch the shared table exists to prevent.
#[cfg(test)]
pub(crate) fn designed_pitch_ramp(t: f64, gradient: PitchGradient) -> Vec4 {
    pitch_ramp_color(t, gradient.sanitized())
}

/// Run `read` over one gradient's table, without copying it.
///
/// The table is memoized on the gradient that built it, in ONE slot: the four
/// knobs hold still except while a control is being dragged, so a single slot
/// hits on essentially every call, and a map keyed on five floats would spend
/// more on hashing than the hit saves. A miss rebuilds — [`PITCH_LUT_N`]
/// entries, each a gamut bisection and an Oklab->sRGB conversion — which is why
/// this is worth caching at all: the per-node draw path asks for a color
/// hundreds of times a frame.
///
/// Thread-local rather than a lock, since the CPU color walk and the scene
/// derive are not the only callers (the offline renderer has its own thread),
/// and a table is small enough that a second thread rebuilding its own copy
/// costs less than either would pay contending for one.
fn with_lut<R>(gradient: PitchGradient, read: impl FnOnce(&[Vec4; PITCH_LUT_N]) -> R) -> R {
    thread_local! {
        static MEMO: std::cell::RefCell<Option<(PitchGradient, [Vec4; PITCH_LUT_N])>> =
            const { std::cell::RefCell::new(None) };
    }
    // Sanitized FIRST, so the key is finite and two gradients that draw the
    // same picture are one cache entry rather than two.
    let gradient = gradient.sanitized();
    MEMO.with(|memo| {
        // The rebuild takes its mutable borrow and gives it back BEFORE `read`
        // runs, which then holds a shared one. `read` is a caller's closure
        // over a table this hands it, so the natural next one asks for another
        // pitch color partway through — and a mutable borrow still standing
        // there turns that into a BorrowMutError on the draw path rather than
        // into a second read of the same table.
        {
            let mut memo = memo.borrow_mut();
            if memo.as_ref().is_none_or(|(key, _)| *key != gradient) {
                let lut = std::array::from_fn(|k| {
                    pitch_ramp_color(k as f64 / (PITCH_LUT_N - 1) as f64, gradient)
                });
                *memo = Some((gradient, lut));
            }
        }
        // The block above fills the slot whenever the key misses, so it is
        // filled here whatever happened.
        let memo = memo.borrow();
        read(&memo.as_ref().expect("memo filled above").1)
    })
}

/// One gradient's ramp, sampled into [`PITCH_LUT_N`] colors evenly spaced over
/// the full `t` range. Both sides read it: the renderer uploads it for the
/// shader to index, and [`pitch_lut_color`] walks it on the CPU.
///
/// Each side maps a pitch to a `t` FIRST and indexes with that, so the
/// gradient's endpoints never reach the table and it stays range-independent.
/// The four knobs are the only thing it varies with — which is why the memo is
/// keyed on those and NOT on the range. A change that folded
/// `darkest_pitch`/`brightest_pitch` into the entries would make that cache
/// wrong, not just stale.
///
/// Hands back a copy, for the renderer, which needs the table as a value to
/// upload. The per-node draw path wants two entries rather than a kilobyte and
/// goes through [`with_lut`] instead.
pub fn pitch_ramp_lut(gradient: PitchGradient) -> [Vec4; PITCH_LUT_N] {
    with_lut(gradient, |lut| *lut)
}

/// Samples in [`hue_circle`], evenly spaced around the hue wheel. The ring
/// that reads it interpolates between entries, so this is how finely the
/// circle's own shape is resolved rather than how many bands are drawn: 96
/// puts a sample every 3.75 degrees, which is under the width of one segment
/// of the ring at the size the pane draws it.
pub const HUE_CIRCLE_N: usize = 96;

/// The whole hue circle at one lightness and chroma — what a gradient's four
/// knobs would give at every hue, not just the arc it takes.
///
/// This is what lets the pane draw the spectrum a gradient has NOT claimed
/// beside the part it has, from the same curve, so the two cannot disagree
/// about what a hue looks like. `chroma` is a fraction of the gamut's maximum
/// exactly as [`PitchGradient::chroma`] is, so the circle is in gamut at every
/// hue for the same reason the ramp is.
///
/// Keyed on the two knobs it actually depends on rather than on a whole
/// gradient: the hue arc is the one being dragged while this is on screen, and
/// keying on the gradient would rebuild the circle every frame of a drag that
/// cannot change it.
///
/// What the key does NOT buy is a free Brightness or Chroma drag. Those move
/// the circle, so every frame of one is a real miss here and another in the
/// pitch table beside it — the two together are (`HUE_CIRCLE_N` +
/// [`PITCH_LUT_N`]) x [`GAMUT_BISECTIONS`] gamut probes, each a Newton solve
/// and an Oklab->sRGB conversion, measuring 412us on the UI thread per frame of
/// such a drag.
///
/// That is the standing price of a chroma that follows the gamut instead of a
/// figure worked out in advance (see [`max_chroma`]), and it is paid only while
/// one of those two knobs is actually moving. Cutting it means either
/// coarsening the bisection, which MOVES THE COLORS every pixel test is pinned
/// to, or caching `max_chroma` against a quantized lightness — a second table
/// with its own staleness to keep honest, for a cost nothing but a drag pays.
pub fn hue_circle(lightness: f32, chroma: f32) -> [Vec4; HUE_CIRCLE_N] {
    /// The circle and the lightness/chroma pair it was built for.
    type Memo = Option<((f32, f32), [Vec4; HUE_CIRCLE_N])>;
    thread_local! {
        static MEMO: std::cell::RefCell<Memo> = const { std::cell::RefCell::new(None) };
    }
    let key = (
        if lightness.is_finite() { lightness.clamp(0.0, 100.0) } else { 0.0 },
        if chroma.is_finite() { chroma.clamp(0.0, 1.0) } else { 0.0 },
    );
    MEMO.with(|memo| {
        let mut memo = memo.borrow_mut();
        if memo.as_ref().is_none_or(|(k, _)| *k != key) {
            let (l, c) = (f64::from(key.0), f64::from(key.1));
            let circle = std::array::from_fn(|k| {
                let h = k as f64 * 360.0 / HUE_CIRCLE_N as f64;
                oklab_srgb(l, h, c * max_chroma(l, h))
            });
            *memo = Some((key, circle));
        }
        memo.as_ref().expect("memo filled above").1
    })
}

/// The pitch gradient, evaluated: [`pitch_ramp_lut`] sampled at `pitch` and
/// interpolated between entries exactly as `pitch_lut_color` in `lattice.wgsl`
/// does. Every pitch-colored shape reaches the ramp through this one walk —
/// the disc, the trail, the piano roll and the melody/bass rings on the CPU,
/// the lit octave glyphs on the GPU.
///
/// It is a LIT pitch that this draws: a sounding glyph stands for a position
/// on the pitch axis rather than for the voice that lit it, and so do the
/// glow's lobes once two octaves sound. The band's ghosts and a solo voice's
/// glow keep the node's own color instead, deliberately — a lone voice keeps
/// its exact color, fixed channel hues included, which the ramp could not
/// reproduce. Do not simplify `octave_glow_color`'s `count < 2u` fallback away
/// on the strength of this function's name.
///
/// One table for all of them is what puts a note's disc and its own lit octave
/// indicator on the same color EXACTLY, rather than to within a tolerance, for
/// a given pitch. (Which pitch each is fed is a separate question: a voice
/// outside the wheel's Range lights the outermost slot on its side, so the
/// disc takes the voice's pitch while the glyph takes the clamped slot's — see
/// `derive`. They differ there because they are naming different pitches, not
/// because two definitions of one pitch's color disagree.)
///
/// The shader can only afford a lookup — a gamut search per fragment
/// is out of reach, and the glow loops call this several times over — so the
/// choice is not "table vs. exact curve" but "one table vs. a table and a
/// curve that disagree". Two shapes sharing an edge is the harshest test of a
/// color match there is, and structural agreement passes it at any table size.
///
/// What the table's size buys is therefore fidelity to the DESIGNED curve
/// ([`pitch_ramp_color`]), never agreement between shapes — see [`PITCH_LUT_N`]
/// for why that fidelity is worth far less per entry than it looks.
pub fn pitch_lut_color(
    pitch: f32,
    darkest_pitch: f32,
    brightest_pitch: f32,
    gradient: PitchGradient,
) -> Vec4 {
    let f = pitch_ramp_t(pitch, darkest_pitch, brightest_pitch) as f32 * (PITCH_LUT_N - 1) as f32;
    // `pitch_ramp_t` clamps into 0..1, so the floor lands inside the table and
    // the last entry pairs with itself at a lerp weight of 0.
    let i0 = f.floor() as usize;
    with_lut(gradient, |lut| {
        lut[i0].lerp(lut[(i0 + 1).min(PITCH_LUT_N - 1)], f - f.floor())
    })
}

/// Ported verbatim from v1 (`editor/color.rs`); the channel policy itself
/// lives in [`ChannelRole`]. Gradient channels are colored by pitch height on
/// the `gradient`'s ramp, spread between `darkest_pitch` and
/// `brightest_pitch` (MIDI note numbers). The fixed per-channel colors below
/// are NOT on that ramp and no gradient setting touches them: a channel color
/// names a voice, and it has to keep meaning the same voice whatever the pitch
/// gradient is set to.
pub fn channel_color(
    channel: u8,
    pitch: f32,
    darkest_pitch: f32,
    brightest_pitch: f32,
    gradient: PitchGradient,
) -> Vec4 {
    match ChannelRole::of(channel) {
        ChannelRole::FixedColor => match channel {
            0 => lch(48.0, 45.0, 32.0),  // red
            1 => lch(65.0, 60.0, 68.0),  // orange
            2 => lch(80.0, 42.0, 83.0),  // yellow
            3 => lch(65.0, 50.0, 120.0), // green
            4 => lch(60.0, 40.0, 280.0), // blue
            5 => lch(50.0, 55.0, 305.0), // purple
            6 => lch(70.0, 30.0, 340.0), // pink
            7 => lch(80.0, 0.0, 0.0),    // white
            _ => lch(0.0, 0.0, 0.0),     // 8: black
        },
        // Through the table, not off the curve direct: the shader has only the
        // table, so this is what puts a disc and the octave glyph drawn on top
        // of it in the same color exactly (see [`pitch_lut_color`]).
        ChannelRole::PitchGradient => {
            pitch_lut_color(pitch, darkest_pitch, brightest_pitch, gradient)
        }
        // Outline voices get a bright neutral (the ring shape is the
        // signal). Ignored never reaches here — the tracker drops it.
        ChannelRole::Outline | ChannelRole::Ignored => Vec4::new(0.85, 0.85, 0.88, 1.0),
    }
}

/// [`max_chroma`], for the test that keeps [`PitchGradient`]'s quoted figures
/// honest. Nothing outside a test may reach the gamut search direct.
#[cfg(test)]
pub(crate) fn max_chroma_for_docs(l_star: f64, h: f64) -> f64 {
    max_chroma(l_star, h)
}

/// The luminance an `L*` names, for the tests that hold a DRAWN color to the
/// curve's own promise rather than to another drawn color. The promise is the
/// whole of what the two-space design buys, and only this side of it is a
/// definition — the other side has a gamut search and a Newton solve in it.
#[cfg(test)]
pub(crate) fn luminance_of(l_star: f64) -> f64 {
    luminance_of_lightness(l_star)
}

/// The luminance one point of the ramp actually reaches, beside the one its
/// `L*` asked for — both in linear light.
///
/// For `the_hybrid_at_the_edges`, which prices [`LUMINANCE_MISS`] against
/// [`LUMINANCE_STEPS`] and cannot do it through the drawn color: an f32 sRGB
/// triple resolves relative luminance to about 1e-7, four orders coarser than
/// the gap between a converged solve and a diverged one.
#[cfg(test)]
pub(crate) fn ramp_luminance(l_star: f64, h: f64, c: f64) -> (f64, f64) {
    let target_y = luminance_of_lightness(l_star);
    let (r, g, b) = RampHue::at(h).linear_srgb(c, target_y);
    (target_y, 0.2126 * r + 0.7152 * g + 0.0722 * b)
}

/// The idle layer's color: the grid color's RGB at full alpha. The grid's
/// alpha is the LINE opacity and doesn't belong to the markers, which have
/// their own presence — so it's dropped here rather than dimming them
/// along with the lines.
pub(crate) fn idle_color(view: &ViewConfig) -> Vec4 {
    let c = view.grid_color;
    Vec4::new(c[0], c[1], c[2], 1.0)
}
