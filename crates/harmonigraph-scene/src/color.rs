//! Node color: the LCh pitch ramp shared by the CPU and the shader LUT,
//! plus channel and idle colors.

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
    // The pitch gradient never needs it — its chroma is a fraction of what
    // [`max_chroma`] says fits, so its colors are inside the gamut by
    // construction. The fixed channel colors below are hand-written LCh and
    // this is what keeps a mistyped one drawable.
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

/// Whether an LCh color is one sRGB can actually show, allowing each channel
/// `slack` past either end of 0..255.
fn in_gamut_within(l: f64, c: f64, h: f64, slack: f64) -> bool {
    let rgb = color_space::Rgb::from(color_space::Lch::new(l, c, h));
    let ok = |v: f64| (-slack..=255.0 + slack).contains(&v);
    ok(rgb.r) && ok(rgb.g) && ok(rgb.b)
}

/// Whether an LCh color is one sRGB can actually show. STRICT: a channel a
/// hair outside 0..255 is out, so a chroma this accepts survives
/// [`lch`]'s clamp untouched and the color drawn is the color asked for.
fn in_gamut(l: f64, c: f64, h: f64) -> bool {
    in_gamut_within(l, c, h, 0.0)
}

/// Bisection steps [`max_chroma`] spends. Each halves the bracket, so 20 over
/// `0..MAX_SEARCH_CHROMA` settles to well under a thousandth of a chroma unit
/// — far finer than the 1/255 the answer is eventually quantized to, and the
/// whole search runs only when the gradient changes.
const GAMUT_BISECTIONS: u32 = 20;

/// Chroma the search brackets from above. sRGB's most saturated color reaches
/// about 133 (blue, near `L*` 32), so nothing is ever cut off by this; it is
/// the bracket rather than a limit.
const MAX_SEARCH_CHROMA: f64 = 200.0;

/// The largest chroma sRGB can show at this lightness and hue.
///
/// Bisection, which needs the in-gamut chromas at a fixed `L*` and hue to be
/// one interval running out from the neutral axis. They are: the gamut is a
/// convex solid in linear RGB and the map into Lab is monotone per channel, so
/// it stays star-shaped about the `L*` axis — every ray out from a neutral
/// crosses the boundary once.
///
/// Bisection is what makes the chroma knob possible at all: the answer varies
/// with BOTH lightness and hue, over a boundary with no closed form, so a
/// gradient free to move either one cannot carry a chroma figure that was
/// worked out in advance.
fn max_chroma(l: f64, h: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, MAX_SEARCH_CHROMA);
    for _ in 0..GAMUT_BISECTIONS {
        let mid = 0.5 * (lo + hi);
        if in_gamut(l, mid, h) {
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
fn pitch_ramp_lch(t: f64, gradient: PitchGradient) -> Vec4 {
    let (l, c, h) = pitch_ramp_lch_coords(t, gradient);
    lch(l, c, h)
}

/// The three coordinates [`pitch_ramp_lch`] converts, before the conversion.
///
/// Split out for [`ramp_sample_in_gamut`], which has to ask about the color
/// that was REQUESTED: [`lch`] clamps, so a color read back out of the table is
/// inside sRGB whatever was asked for, and a clamp cannot also be the check on
/// whether it was needed.
///
/// Takes an already-sanitized gradient and does not re-sanitize. The invariant
/// has boundaries rather than a scattering of owners — [`with_lut`] applies it
/// once for a whole table (and keys the memo on the result),
/// [`designed_pitch_ramp`] does it for the test path, and
/// [`PitchGradient::lightness_and_hue`] holds up its own public end. Only
/// `chroma` is read raw here, which is what lets the gamut test hand in the
/// out-of-range fraction a control cannot produce.
fn pitch_ramp_lch_coords(t: f64, gradient: PitchGradient) -> (f64, f64, f64) {
    let (l, h) = gradient.lightness_and_hue(t);
    (l, f64::from(gradient.chroma) * max_chroma(l, h), h)
}

/// Whether the curve's ask at `t` is a color sRGB can actually show, put to the
/// strict predicate instead of to the clamped result. For
/// `the_gradient_is_in_gamut_and_flat_when_its_ramp_is`, which cannot learn
/// anything by reading the table back: every entry there has already been
/// clamped into range by [`lch`].
///
/// Deliberately NOT sanitizing, so the test can hand in a chroma past 1.0 and
/// watch this say no — a check whose failing case is unreachable is not a
/// check.
///
/// Half a quantization step of slack, where [`max_chroma`]'s own predicate
/// takes none. The strictness there is what keeps the clamp idle: a bisection
/// that stopped a hair OUTSIDE would hand back a chroma [`lch`] then has to
/// clip. Asked the other question — was the clamp needed? — that same
/// strictness reads pure white as out of gamut, since `Lch(100, 0, h)` converts
/// a whisker past 255 and the sweep reaches `L*` 100 wherever a steep ramp
/// flattens against the top of the axis. A clamp that moves a channel by less
/// than half a byte moves no byte at all.
#[cfg(test)]
pub(crate) fn ramp_sample_in_gamut(t: f64, gradient: PitchGradient) -> bool {
    let (l, c, h) = pitch_ramp_lch_coords(t, gradient);
    in_gamut_within(l, c, h, 0.5)
}

/// The designed curve, for the test that pins how closely [`pitch_ramp_lut`]
/// tracks it. Nothing in the draw path may call this — going off the curve
/// direct is precisely the mismatch the shared table exists to prevent.
#[cfg(test)]
pub(crate) fn designed_pitch_ramp(t: f64, gradient: PitchGradient) -> Vec4 {
    pitch_ramp_lch(t, gradient.sanitized())
}

/// Run `read` over one gradient's table, without copying it.
///
/// The table is memoized on the gradient that built it, in ONE slot: the four
/// knobs hold still except while a control is being dragged, so a single slot
/// hits on essentially every call, and a map keyed on five floats would spend
/// more on hashing than the hit saves. A miss rebuilds — [`PITCH_LUT_N`]
/// entries, each a gamut bisection and an LCh->sRGB conversion — which is why
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
                    pitch_ramp_lch(k as f64 / (PITCH_LUT_N - 1) as f64, gradient)
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
/// [`PITCH_LUT_N`]) x [`GAMUT_BISECTIONS`] LCh->sRGB conversions, measuring
/// 454us on the UI thread per frame of such a drag.
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
                lch(l, c * max_chroma(l, h), h)
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
/// The shader can only afford a lookup — an LCh->sRGB conversion per fragment
/// is out of reach, and the glow loops call this several times over — so the
/// choice is not "table vs. exact curve" but "one table vs. a table and a
/// curve that disagree". Two shapes sharing an edge is the harshest test of a
/// color match there is, and structural agreement passes it at any table size.
///
/// What the table's size buys is therefore fidelity to the DESIGNED curve
/// ([`pitch_ramp_lch`]), never agreement between shapes — see [`PITCH_LUT_N`]
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
/// the `gradient`'s LCh ramp, spread between `darkest_pitch` and
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

/// The idle layer's color: the grid color's RGB at full alpha. The grid's
/// alpha is the LINE opacity and doesn't belong to the markers, which have
/// their own presence — so it's dropped here rather than dimming them
/// along with the lines.
pub(crate) fn idle_color(view: &ViewConfig) -> Vec4 {
    let c = view.grid_color;
    Vec4::new(c[0], c[1], c[2], 1.0)
}
