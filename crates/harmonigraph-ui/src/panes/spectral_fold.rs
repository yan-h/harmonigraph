//! Folding the analyzer's spectrum onto the lattice: per node, per octave
//! slot, how much power is sounding at exactly that pitch.
//!
//! The lattice's own coordinate system is the harmonic series. A partial sits
//! at an EXACT rational multiple of its fundamental, so on a 7-limit lattice
//! partials do not approximately map to nodes — they land on them, and folded
//! to pitch class the first sixteen harmonics of any note occupy six nodes
//! (the fundamental's own for 1/2/4/8/16, +1 threes for 3/6/12, +1 fives for
//! 5/10, +1 sevens for 7/14, +2 threes for 9, +1 threes +1 fives for 15). A
//! timbre is therefore a CONSTELLATION anchored at its fundamental's node,
//! whose brightness distribution is the timbre itself. The octave wheel does
//! the rest for free: a wedge's angle already means an absolute pitch, so
//! folding per slot rather than to one scalar says which REGISTER each partial
//! sounds in.
//!
//! Not a pane, though it lives beside them: it is a post-pass the Lattice pane
//! runs, and it is made of the two panes it sits between — the analyzer's
//! spectrum and the lattice's own wheel. Named `spectral_fold` and not `fold`
//! because [`crate::fold`] is the dock's pane folding, which has nothing to do
//! with any of this.
//!
//! ## Weight, don't gate
//!
//! Nothing here detects anything. There are no peaks, no thresholds and no
//! discrete partials: every value is a kernel-weighted mean of power over a
//! window of the log-pitch grid, so a partial drifting off a node dims
//! smoothly instead of switching off, and ±15¢ of vibrato reads as breathing.
//! `harmonigraph_core::spectrum`'s own doc records what the alternative cost
//! the spectrogram — a peak-only fill drew broadband sound as flickering
//! speckle, because broadband sound has no peaks to find.
//!
//! What keeps noise OFF the lattice is therefore not a detector but the noise
//! FLOOR: a local estimate is subtracted before folding, so only energy
//! concentrated above its own neighbourhood survives, which is the definition
//! of a partial. Genuinely pitched noise — a cymbal, a snare wash — still
//! lights a region dimly, and that is the correct display: those frequencies
//! are genuinely sounding.
//!
//! ## The estimator, and what it costs
//!
//! The floor is a MEDIAN over ±150¢ of the grid (300¢ across), evaluated every
//! 8 buckets and interpolated between. A median because a partial occupies a
//! small fraction of that window and cannot move it, where a mean would be
//! lifted by the very thing it is meant to measure; 300¢ because it is wide
//! enough to hold a partial and its skirts at any pitch the plugin is aimed at
//! and narrow enough to follow a spectral envelope. It is subtracted in POWER
//! rather than in dB — total power is floor plus partial, and that is the
//! arithmetic that undoes it.
//!
//! Two costs worth stating rather than discovering:
//!
//! - **Down low the floor eats the fundamental.** An FFT bin is a constant
//!   number of Hz, so at 65 Hz (C2) it is 151¢ wide and one Hann main lobe
//!   spans about 600¢ — wider than the whole median window, which then sits
//!   INSIDE the lobe and subtracts most of it. The pitch class still lights,
//!   from the 2nd harmonic up, and it lights the right node; what goes dark is
//!   its lowest wedge. That is the same limit #350 names under "low
//!   fundamentals blur", showing up here as a floor rather than as a smear.
//! - **Up high the floor rises between partials.** Harmonics crowd together in
//!   log pitch — the 15th and 16th are 112¢ apart — so a 300¢ window holds
//!   three of them and the median lands in a valley rather than under the
//!   whole group. That reads as the top of the constellation being a little
//!   dimmer than its true power, never as a partial vanishing.
//!
//! ## Why a MEAN and not a sum
//!
//! The kernel is normalized by its own weights, so the fold answers in the
//! analyzer's own units: a bucket reading power P surrounded by more of the
//! same folds to P, exactly as
//! [`power_mean`](super::spectral::spectrogram::power_mean) answers for the
//! Spiral pane. That is what lets the result go through the shared
//! [`loudness`] curve and mean the same thing
//! there as everywhere else, and it is what makes the width bar a TOLERANCE
//! rather than a gain — a plain sum would brighten the whole lattice every time
//! the bar was dragged right.
//!
//! It is also what makes a sub-bucket kernel behave: at 1¢ the kernel is a
//! third of a bucket wide, so it lands between two of them as often as on one,
//! and a mean of the two neighbours is the same answer either way where a sum
//! would halve.
//!
//! The cost is at the wide end: a kernel much wider than a partial's main lobe
//! averages the valleys either side into it and reads dimmer. Since the width
//! is what admits a DETUNED partial, and a detuned partial is meant to read
//! dim, that failure points the right way.
//!
//! ## No envelope of its own
//!
//! [`AudioSpectrum::display`](crate::AudioSpectrum::display) is already
//! EMA-smoothed on the hop grid by the Analyzer's own Smoothing control, so the
//! fold inherits an attack/decay that is on the sample grid — deterministic,
//! and shared with every other pane, so a hop that shimmers here shimmers on
//! the Spectral pane too and one control settles both. A second envelope here
//! would need per-node state across frames, and the lattice is drawn TWICE in
//! an editor frame (the docked pane and the Render preview's copy), so that
//! state would tick at twice the rate in one shell and once in another — the
//! offline renderer's determinism is exactly what that breaks. Stateless is
//! therefore not merely simpler here, it is the only version that draws the
//! same frame in all three places.

use harmonigraph_core::spectrum::{
    BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI,
};
use harmonigraph_scene::{
    OctaveLayout, Scene, OCTAVE_SLOTS, SPECTRAL_WIDTH_MAX, SPECTRAL_WIDTH_MIN,
};

use super::spectral::axes::loudness;
use crate::spectrum::SpectrumBuckets;
use crate::SharedState;

/// Cents one analyzer bucket spans: 3.125, the grid everything below counts in.
const CENTS_PER_BUCKET: f32 = 1200.0 / (12 * BINS_PER_SEMITONE) as f32;

/// How far the kernel reaches, in standard deviations. Past three the weight is
/// under 1.1% and what it buys is buckets to walk.
const KERNEL_REACH: f32 = 3.0;

/// Buckets between noise-floor estimates: 8, which is 25¢.
///
/// The floor is a slowly-moving thing (it is a median over 300¢), so estimating
/// it at every bucket would be 8 times the sorting for a curve that has not
/// moved in between. What is between two estimates is interpolated — in POWER,
/// which over 25¢ is within a fraction of a dB of interpolating in dB and costs
/// no logarithm per bucket.
const FLOOR_STEP: usize = 8;

/// Half the median window, in buckets: 48, which is 150¢ either side.
const FLOOR_HALF: usize = 48;

/// Buckets between the samples the median is taken over. The window holds 97
/// buckets and the median of every fourth one is the same number to well within
/// the noise it is measuring, at a quarter of the sorting.
const FLOOR_SAMPLE_STEP: usize = 4;

/// How many samples that leaves: 25, an odd count so the median is an element
/// rather than a pair averaged.
const FLOOR_SAMPLES: usize = 2 * (FLOOR_HALF / FLOOR_SAMPLE_STEP) + 1;

/// Floor estimates kept, covering the whole axis with one to spare so the
/// interpolation always has a pair.
const FLOOR_POINTS: usize = SPECTRUM_BINS.div_ceil(FLOOR_STEP) + 1;

/// Octave slots the analyzer's pitch axis (MIDI 15.49..135.08) can hold an
/// octave of a pitch class in: slot `s` is the octave whose C is MIDI `12 * s`,
/// so this is C0 up to C10. Both ends are only PARTLY in range — which of a
/// class's octaves are is settled per pitch, by the bounds check in
/// [`Fold::slot_power`] — so these are where to start and stop looking rather
/// than what is audible.
const LOWEST_AUDIBLE_SLOT: i32 = 1;
/// See [`LOWEST_AUDIBLE_SLOT`].
const HIGHEST_AUDIBLE_SLOT: i32 = 11;

/// One node's spectral light: how much power sounds at each of its octaves.
///
/// Power in the analyzer's own units throughout, NOT levels — the mapping to
/// what is drawn is [`loudness`]'s, and doing it here would put a second copy
/// of "how loud is loud" in the tree. It is also what lets the tests state the
/// constellation in the dB the harmonic series predicts.
pub(crate) struct NodeLight {
    /// Power folded onto each drawn octave slot. Slots the wheel does not draw
    /// are 0; energy from octaves past either end of the wheel is folded onto
    /// the outermost slice on its side, exactly as a MIDI note past the end is.
    pub(crate) octaves: [f32; OCTAVE_SLOTS],
    /// The node body's power: the sum of the above, which is what makes a
    /// timbre's constellation a set of node brightnesses rather than a set of
    /// rings.
    pub(crate) total: f32,
    /// Where this node's energy SITS, as an absolute MIDI pitch: the
    /// power-weighted mean of its lit slots' own pitches.
    ///
    /// Two things need a single pitch for a node whose energy is spread over
    /// octaves — the tilt in [`loudness`], and the colour off the pitch ramp —
    /// and the loudest slot's pitch is the tempting answer that cannot be used:
    /// it JUMPS an octave the moment the argmax changes hands, which on a
    /// sustained note is a hue and a brightness stepping while nothing audible
    /// moved. A weighted mean is continuous in the power, and on a node with
    /// one lit octave it is exactly that octave's pitch — which is the same
    /// answer the shader's own glow falls back to for a solo voice.
    pub(crate) pitch: f32,
}

/// The frame's fold: the kernel-weighted mean excess power at every bucket of
/// the analyzer's grid, with a node's octave then a lookup into it.
///
/// **The whole fold is done once per lattice draw, not once per node**, and
/// that is what bounds its cost. The kernel is the same shape wherever it is
/// centered, so folding it over the grid ONCE answers every node and every
/// octave slot at a stroke — the alternative walks its own window per node per
/// slot, which at a deep window (1365 nodes, eleven octaves) is 1.5 million
/// bucket reads and was measured at 2.7 ms a frame, twice over in an editor
/// frame that draws the lattice for the pane and again for the Render preview.
/// This is 3828 buckets whatever the extents are, and the per-node work is
/// eleven interpolated reads.
///
/// What it costs is a fraction of a bucket's worth of blur: a node's own pitch
/// falls BETWEEN buckets, and where the per-node version centered its kernel
/// exactly there, this reads the two nearest smoothed buckets and interpolates.
/// The smoothed curve is a Gaussian convolution, so it is smooth on the scale of
/// the kernel and the error is a fraction of a percent at any width the bar can
/// reach.
pub(crate) struct Fold {
    /// One value per analyzer bucket: how much power stands above the local
    /// noise floor there, averaged under the kernel. Boxed rather than inline —
    /// 15 KB is not a thing to hand back by value from a constructor.
    smoothed: Box<[f32; SPECTRUM_BINS]>,
}

impl Fold {
    /// Measure the frame's floor and fold the kernel over it. `width` is the
    /// kernel's standard deviation in cents
    /// ([`ViewConfig::spectral_width`](harmonigraph_scene::ViewConfig)).
    ///
    /// The width is clamped here as well as in `ViewConfig::sanitize`, and not
    /// as a second opinion about the range: this is the one place a zero or a
    /// NaN would divide, and the shells reach the drawing code by more routes
    /// than the persist door (a take replay, the offline renderer's layout, a
    /// standalone harness), so the guard belongs where the division is.
    pub(crate) fn measure(levels: &SpectrumBuckets, width: f32) -> Fold {
        let width = if width.is_finite() {
            width.clamp(SPECTRAL_WIDTH_MIN, SPECTRAL_WIDTH_MAX)
        } else {
            SPECTRAL_WIDTH_MIN
        };
        let sigma = width / CENTS_PER_BUCKET;

        // The floor, then the excess over it, both per bucket. One pass each
        // rather than the floor re-interpolated inside the kernel's own loop,
        // where every bucket would be read once per tap.
        let mut floor = [0.0f32; FLOOR_POINTS];
        // One scratch window reused across the estimates: `select_nth` sorts in
        // place, so it has to be a copy of the samples rather than a view of
        // them, and 480 allocations a frame is 480 more than this needs.
        let mut window = [0.0f32; FLOOR_SAMPLES];
        for (i, out) in floor.iter_mut().enumerate() {
            let center = (i * FLOOR_STEP) as isize;
            for (j, sample) in window.iter_mut().enumerate() {
                let offset = (j * FLOOR_SAMPLE_STEP) as isize - FLOOR_HALF as isize;
                // Clamped rather than skipped, so every estimate is a median of
                // the same number of samples: a window shortened at the axis
                // ends would make the floor there a median of a different
                // statistic, which is the one place nothing would notice.
                let bucket = (center + offset).clamp(0, SPECTRUM_BINS as isize - 1) as usize;
                *sample = levels[bucket];
            }
            *out = *window.select_nth_unstable_by(FLOOR_SAMPLES / 2, f32::total_cmp).1;
        }
        let mut excess = Box::new([0.0f32; SPECTRUM_BINS]);
        for (bucket, out) in excess.iter_mut().enumerate() {
            // Interpolated between the estimates in POWER, and floored at zero:
            // what is under its own neighbourhood is not a negative partial.
            let x = bucket as f32 / FLOOR_STEP as f32;
            let i = (x as usize).min(FLOOR_POINTS - 2);
            let t = x - i as f32;
            *out = (levels[bucket] - (floor[i] + (floor[i + 1] - floor[i]) * t)).max(0.0);
        }

        // Half the kernel, sampled at whole buckets — it is symmetric, so the
        // other half is the same numbers read backwards. At least one tap
        // whatever the width: below about 3¢ the kernel is narrower than a
        // bucket, and a fold with no taps at all would answer zero everywhere.
        let reach = ((KERNEL_REACH * sigma).ceil() as usize).max(1);
        let taps: Vec<f32> = (0..=reach)
            .map(|d| {
                let x = d as f32 / sigma;
                (-0.5 * x * x).exp()
            })
            .collect();

        let mut smoothed = Box::new([0.0f32; SPECTRUM_BINS]);
        for (bucket, out) in smoothed.iter_mut().enumerate() {
            let first = bucket.saturating_sub(reach);
            let last = (bucket + reach).min(SPECTRUM_BINS - 1);
            let (mut sum, mut weight) = (0.0f32, 0.0f32);
            for (i, &power) in excess[first..=last].iter().enumerate() {
                let w = taps[(first + i).abs_diff(bucket)];
                sum += w * power;
                weight += w;
            }
            // Normalized by the weights that were actually used, so a bucket at
            // the axis ends — where the window is cut short — reads a mean of
            // what is there rather than one diluted by the half that is missing.
            *out = if weight > 0.0 { sum / weight } else { 0.0 };
        }
        Fold { smoothed }
    }

    /// The kernel-weighted mean power above the floor at absolute MIDI `pitch`,
    /// or `None` where the analyzer's axis does not reach that pitch.
    fn slot_power(&self, pitch: f32) -> Option<f32> {
        if !(SPECTRUM_MIN_MIDI..=SPECTRUM_MAX_MIDI).contains(&pitch) {
            return None;
        }
        let x = (pitch - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32;
        let i = (x.max(0.0) as usize).min(SPECTRUM_BINS - 2);
        let t = (x - i as f32).clamp(0.0, 1.0);
        Some(self.smoothed[i] + (self.smoothed[i + 1] - self.smoothed[i]) * t)
    }

    /// Fold the spectrum onto one node — a lattice position whose pitch class
    /// is `cents` (0..1200) — laid out on `layout`'s wheel.
    pub(crate) fn node(&self, layout: &OctaveLayout, cents: f32) -> NodeLight {
        let (low, high) = layout.slots(cents);
        let mut octaves = [0.0f32; OCTAVE_SLOTS];
        for slot in LOWEST_AUDIBLE_SLOT..=HIGHEST_AUDIBLE_SLOT {
            let Some(power) = self.slot_power(layout.slot_pitch(slot, cents)) else {
                continue;
            };
            // Onto the slice that DRAWS this octave: into the wheel first, so
            // energy past either end lands on the outermost slice on its side,
            // and then into the packing, which holds the MIDI octaves and
            // nothing else. The same two clamps `derive_scene` puts a played
            // note through, for the same reasons.
            let drawn = slot.clamp(low, high).clamp(0, OCTAVE_SLOTS as i32 - 1) as usize;
            // The strongest octave folded here, not their sum: a slice standing
            // in for several octaves is saying "the loudest thing out this way
            // is this loud", which is what the MIDI rule says too. Summing
            // would let a handful of inaudible octaves add up to a bright
            // outermost wedge that no single pitch is responsible for.
            octaves[drawn] = octaves[drawn].max(power);
        }
        let total: f32 = octaves.iter().sum();
        let mut pitch = 0.0;
        if total > 0.0 {
            for (slot, &power) in octaves.iter().enumerate() {
                pitch += power * layout.slot_pitch(slot as i32, cents);
            }
            pitch /= total;
        }
        NodeLight { octaves, total, pitch }
    }
}

/// Relight `scene`'s nodes from the analyzer instead of from MIDI.
///
/// A post-pass over a scene derived exactly as it always is, which is the whole
/// of the seam: the geometry, the wheel, the grid, the melody/bass marks, the
/// trail and the camera are `derive_scene`'s answers untouched, and what is
/// overwritten is the four things a VOICE decides — how lit the node is, which
/// of its octaves are sounding, what colour it wears, and whether it clears its
/// gutter.
///
/// The colour has to come along, though the level is what the toggle is about:
/// a node's colour is the ramp at its VOICE's pitch, so a node with no voice on
/// it holds the idle grey, and a spectrally lit node left holding it would draw
/// the constellation in the colour of the grid. The shader blends the octaves'
/// own hues around a node lighting two or more of them and falls back to this
/// colour for one — so what is written here is the same pitch the fold already
/// had to settle for the tilt, and a one-octave node comes out the colour that
/// octave would have been.
///
/// No audio flowing is DARK rather than a fallback to MIDI. A source toggle
/// that quietly handed the lattice back when the input went quiet would read as
/// the toggle failing, and the input going quiet is the one thing the picture
/// should say plainly.
pub(crate) fn light_from_spectrum(scene: &mut Scene, state: &SharedState, now: f64) {
    let cfg = state.spectrum_config;
    let gutter = state.view.sevens_gutter.clamp(0.0, 0.5);
    let width = state.view.spectral_width;
    let fold = state.spectrum.display(now).map(|levels| Fold::measure(levels, width));
    let layout = scene.octave_layout;
    for node in &mut scene.nodes {
        node.activation = 0.0;
        node.octaves = [0.0; OCTAVE_SLOTS];
        // Nothing here departs: a partial has no key to come up, so the flag
        // that means "this level is on its way out" has no answer but no. Its
        // one reader reserves a label's brightness ahead of a trail record, and
        // a trail is the MIDI history's — see `label_strength`.
        node.departing = false;
        node.gutter = 0.0;
        let Some(fold) = &fold else { continue };
        let light = fold.node(&layout, node.cents);
        if light.total <= 0.0 {
            continue;
        }
        node.activation = loudness(&cfg, light.total, light.pitch);
        for (slot, level) in node.octaves.iter_mut().enumerate() {
            let pitch = layout.slot_pitch(slot as i32, node.cents);
            *level = loudness(&cfg, light.octaves[slot], pitch);
        }
        if node.activation > 0.0 {
            node.color = harmonigraph_scene::pitch_lut_color(
                light.pitch,
                state.frame_params.darkest_pitch,
                state.frame_params.brightest_pitch,
                state.view.pitch_gradient,
            );
            node.gutter = gutter;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::fresh;
    use harmonigraph_core::{spectrum::midi_to_hz, LatticePos, NoteEvent, Tuning};
    use harmonigraph_scene::{derive_scene, octave_layout, ViewConfig};

    const SR: f32 = 48_000.0;

    /// A second of audio, which is 125 analyzer hops — long enough that the
    /// display buckets are the steady spectrum and not a window still filling.
    const SECONDS: f32 = 1.0;

    /// A sawtooth at `midi`: every harmonic up to Nyquist at amplitude `1/k`,
    /// so harmonic `k` carries power `1/k²` and the whole constellation is
    /// PREDICTED rather than measured — the fundamental's node sums 1, 4, 16…,
    /// the fifth's sums 9, 36…, and the ratios between them are what the tests
    /// below check the fold against.
    ///
    /// A real saw and not the first six harmonics, deliberately: the harmonics
    /// this panel does NOT claim (11 and 13 fall outside the 7-limit, and
    /// everything above 16 crowds) are exactly what a "nothing else lights"
    /// claim has to be made against.
    fn sawtooth(midi: f32) -> Vec<f32> {
        let f = midi_to_hz(midi);
        (0..(SECONDS * SR) as usize)
            .map(|i| {
                let t = i as f32 / SR;
                let mut sum = 0.0;
                let mut k = 1.0f32;
                while k * f < SR * 0.45 {
                    sum += (std::f32::consts::TAU * k * f * t).sin() / k;
                    k += 1.0;
                }
                sum
            })
            .collect()
    }

    /// A full-scale sine at `midi`, the signal the analyzer's 0 dB is defined
    /// as.
    fn sine(midi: f32) -> Vec<f32> {
        let f = midi_to_hz(midi);
        (0..(SECONDS * SR) as usize)
            .map(|i| (std::f32::consts::TAU * f * i as f32 / SR).sin())
            .collect()
    }

    /// White noise from a fixed seed — broadband, with no partials in it at
    /// all, which is the input the floor estimate exists for. Its own LCG
    /// rather than a crate: the tree has no rand dependency, and a test signal
    /// has to be the same one on every machine.
    fn noise() -> Vec<f32> {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        (0..(SECONDS * SR) as usize)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 40) as f32 / (1u32 << 23) as f32 - 1.0
            })
            .collect()
    }

    /// One signal analyzed, ready to fold nodes out of.
    struct Bench {
        levels: SpectrumBuckets,
        layout: OctaveLayout,
        tuning: Tuning,
    }

    impl Bench {
        /// Analyze `samples` at the fresh view's own octave wheel, in JUST
        /// intonation — which is the material this panel is aimed at, and the
        /// whole reason the constellation is exact: a partial of a just-tuned
        /// note lands ON its node rather than near it.
        fn on(samples: &[f32]) -> Bench {
            let mut state = fresh();
            // The display's EMA is the Analyzer's Smoothing control, and it is
            // an inheritance the fold is glad of in the plugin and cannot use
            // here: it would make every number below a function of how many
            // hops the fixture happened to push.
            state.spectrum_config.smoothing = 0.0;
            let cfg = state.spectrum_config;
            state.spectrum.push_samples(samples, 1, SR, 1.0, &cfg);
            let levels = *state.spectrum.display(1.0).expect("a second of audio is enough");
            let view = ViewConfig::default();
            Bench {
                levels,
                layout: octave_layout(
                    view.octave_count,
                    view.octave_center,
                    view.octave_extras,
                    view.octave_extra_size,
                    view.octave_extra_blend,
                ),
                tuning: Tuning::just(),
            }
        }

        fn light(&self, width: f32, pos: LatticePos) -> NodeLight {
            self.at_class(width, self.tuning.pitch_class(pos).to_cents())
        }

        /// The same for a pitch class named directly, which is what the
        /// selectivity tests want: they ask how far apart two classes have to
        /// be, and a lattice position is a roundabout way of naming one.
        fn at_class(&self, width: f32, cents: f32) -> NodeLight {
            Fold::measure(&self.levels, width).node(&self.layout, cents)
        }

        /// The node body's power in dB, which is where the harmonic series
        /// states itself.
        fn db(&self, width: f32, pos: LatticePos) -> f32 {
            power_db(self.light(width, pos).total)
        }
    }

    fn power_db(power: f32) -> f32 {
        10.0 * power.max(1e-12).log10()
    }

    /// The fresh width, and the one every claim about the constellation is
    /// measured at.
    const NARROW: f32 = 10.0;

    /// The six nodes the first sixteen harmonics fold onto, in the order their
    /// power puts them, with the dB each sits under the fundamental's node —
    /// read off `1/k²` and the octave sums, not off the code.
    ///
    /// The fundamental's node totals `1 + 1/4 + 1/16 + … = 4/3`; the fifth's
    /// `(1/9)(4/3)`, the third's `(1/25)(4/3)`, the seventh's `(1/49)(4/3)`,
    /// the ninth's `(1/81)(4/3)` and the fifteenth's `(1/225)(4/3)`.
    const CONSTELLATION: [(&str, LatticePos, f32); 6] = [
        ("the fundamental", LatticePos { threes: 0, fives: 0, sevens: 0 }, 0.0),
        ("+1 threes", LatticePos { threes: 1, fives: 0, sevens: 0 }, -9.54),
        ("+1 fives", LatticePos { threes: 0, fives: 1, sevens: 0 }, -13.98),
        ("+1 sevens", LatticePos { threes: 0, fives: 0, sevens: 1 }, -16.90),
        ("+2 threes", LatticePos { threes: 2, fives: 0, sevens: 0 }, -19.08),
        ("+1 threes +1 fives", LatticePos { threes: 1, fives: 1, sevens: 0 }, -23.52),
    ];

    /// How far off the harmonic series' own prediction a node is allowed to
    /// fold, in dB, and it is not one number: the four the issue's check names
    /// land inside 2 dB, and the two faintest run progressively dim, by 2 and 4.
    ///
    /// The reason is in the module docs and it is one-directional, which is why
    /// the slack grows down the list rather than being loosened for all six: the
    /// faint end of a saw's constellation lives HIGH, where harmonics crowd to
    /// within a hundred cents of each other and the noise floor's own median
    /// window holds three of them at once. That lifts the floor into the group
    /// and takes a few dB off it. It cannot go the other way — nothing there
    /// makes a node brighter than its partials — so a failure past this is a
    /// fold that has started counting energy twice.
    const CONSTELLATION_SLACK: [f32; 6] = [2.0, 2.0, 2.0, 2.0, 2.5, 4.5];

    /// A sawtooth draws the harmonic series: the constellation's six nodes come
    /// out in the order `1/k²` predicts, at the levels it predicts.
    ///
    /// Measured on a saw at C3 and not at the C2 the issue's check names,
    /// because C3 is where the analyzer resolves BOTH ends of the picture. At
    /// C2 one FFT bin is 151¢ and the fundamental's main lobe is wider than the
    /// noise floor's own window, so the floor sits inside the lobe and takes
    /// 4 dB off the node the other five are measured against — the whole
    /// constellation then comes out 3 dB compressed. The measurement is on the
    /// issue; what it means is that this panel reads a bass note's SHAPE, not
    /// its proportions.
    #[test]
    fn a_sawtooth_draws_the_harmonic_series_as_a_constellation() {
        let bench = Bench::on(&sawtooth(48.0));
        let root = bench.db(NARROW, CONSTELLATION[0].1);
        let mut previous = f32::INFINITY;
        for (i, (name, pos, want)) in CONSTELLATION.into_iter().enumerate() {
            let got = bench.db(NARROW, pos) - root;
            assert!(
                (got - want).abs() < CONSTELLATION_SLACK[i],
                "{name} folded to {got:.2} dB under the fundamental's node, \
                 not the {want:.2} the harmonic series predicts",
            );
            // The ORDER as well as the levels, which is the half a per-node
            // tolerance cannot state: the constellation is read as a shape, and
            // a shape is which node is brighter than which.
            assert!(got < previous, "{name} is no dimmer than the node before it");
            previous = got;
        }
    }

    /// A node the analyzer can tell apart from everything sounding is DARK, and
    /// a semitone is that distance everywhere the plugin looks: a lone tone
    /// leaves the class a semitone off it more than 30 dB down.
    ///
    /// Stated in pitch-class distance rather than as "nothing outside the
    /// constellation lights", which is not true of a lattice and is worth being
    /// exact about. Two reasons, neither a defect in the fold:
    ///
    /// - A real sawtooth's 11th and 13th harmonics fall outside the 7-limit,
    ///   and they are genuinely sounding — a node near either lights, correctly.
    /// - At the fresh extents the just lattice is FINER than the analyzer: 21
    ///   fifths by 13 thirds puts a node every few cents, and pairs a syntonic
    ///   comma apart (21.5¢) are closer than an 8192-point window resolves below
    ///   about a kilohertz. Both of the pair light, and they are far apart ON
    ///   SCREEN. That is the panel's sharpness limit and it is the ANALYZER's,
    ///   not the kernel's — which is what the second half of this test pins, so
    ///   that a later "the width bar should be tighter" cannot be believed.
    #[test]
    fn a_node_the_analyzer_resolves_apart_stays_dark() {
        for base in [48.0f32, 60.0, 84.0, 96.0] {
            let bench = Bench::on(&sine(base));
            let on = power_db(bench.at_class(NARROW, base.rem_euclid(12.0) * 100.0).total);
            let off = power_db(bench.at_class(NARROW, (base + 1.0).rem_euclid(12.0) * 100.0).total);
            assert!(
                on - off > 30.0,
                "a tone at {base} left the class a semitone up only {:.2} dB down",
                on - off,
            );
        }
        // And the resolution that costs: 20¢ off a tone at C4 is barely dimmer,
        // where 20¢ off one at C6 is well down — the same kernel both times, so
        // what changed is what the FFT can see. One bin spans 38¢ at C4 and 10¢
        // at C6.
        let low = Bench::on(&sine(60.0));
        let high = Bench::on(&sine(84.0));
        let near = |bench: &Bench, base: f32| {
            power_db(bench.at_class(NARROW, base.rem_euclid(12.0) * 100.0).total)
                - power_db(bench.at_class(NARROW, base.rem_euclid(12.0) * 100.0 + 20.0).total)
        };
        assert!(near(&low, 60.0) < 3.0, "the analyzer resolved 20¢ at C4, which it cannot");
        assert!(near(&high, 84.0) > 5.0, "20¢ at C6 is inside one bin, which it is not");
    }

    /// The wheels say which REGISTER each partial sounds in — the reading the
    /// octave fold buys and a single scalar per node could not.
    ///
    /// A saw on C2 lights C wedges from C2 upward, a G wedge no lower than G3
    /// (its lowest partial is the 3rd harmonic, a twelfth up) and an E wedge no
    /// lower than E4 (the 5th, two octaves and a third up). The slots below
    /// each of those are the ones that must stay dark: they are the octaves a
    /// harmonic series cannot reach downward, and lighting one would mean the
    /// fold had folded a partial onto the wrong octave.
    #[test]
    fn the_wedges_climb_in_register() {
        let bench = Bench::on(&sawtooth(36.0));
        // Slot `s` is the octave whose C is MIDI `12 * s`, so C2 is slot 3.
        for (name, pos, lowest) in [
            ("C", LatticePos::new(0, 0, 0), 3),
            ("G", LatticePos::new(1, 0, 0), 4),
            ("E", LatticePos::new(0, 1, 0), 5),
        ] {
            let light = bench.light(NARROW, pos);
            let loudest = light
                .octaves
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(slot, _)| slot)
                .expect("a node has slots");
            assert_eq!(loudest, lowest, "the loudest {name} wedge is not its lowest partial's");
            // Two octaves up from the lowest one still sound, so the wedges
            // really do climb rather than the node lighting one of them.
            for slot in lowest..=lowest + 2 {
                assert!(
                    light.octaves[slot] > 0.0,
                    "{name} is dark at slot {slot}, which its harmonics reach",
                );
            }
            // A tenth of a percent of the loudest wedge, in power: an octave
            // that no partial reaches is not exactly zero — the analyzer's own
            // skirts land everywhere — and what the claim is about is that it
            // carries nothing anyone can see.
            let quiet = light.octaves[lowest] * 1e-3;
            for slot in 0..lowest {
                assert!(
                    light.octaves[slot] < quiet,
                    "{name} lights slot {slot}, an octave below every partial it has",
                );
            }
        }
    }

    /// Noise does not light the lattice. Broadband sound has no partials, so
    /// what the floor subtraction leaves is what did not stand above its own
    /// neighbourhood — which for white noise is a whisper.
    ///
    /// Against a TONE rather than against an absolute level, because an
    /// absolute one is a claim about how loud the fixture's noise happens to
    /// be: what has to hold is that a pitch and a hiss of the same power are
    /// nowhere near each other on the lattice.
    #[test]
    fn noise_alone_does_not_light_the_lattice() {
        let tone = Bench::on(&sine(60.0)).db(NARROW, LatticePos::new(0, 0, 0));
        let hiss = Bench::on(&noise());
        // The whole fresh window, and the LOUDEST node in it: a floor estimate
        // is a statistic and 273 nodes is 273 draws from it, so what a haze
        // looks like on screen is set by its luckiest node rather than by its
        // average one. Full-scale noise leaves that node 26 dB under a
        // full-scale tone, which is a whisper against the default 60 dB window
        // and is the "dim floor" the design admits to rather than nothing at
        // all — the honest answer, since those frequencies really are sounding.
        for pos in ViewConfig::default().visible_positions() {
            let got = hiss.db(NARROW, pos);
            assert!(
                got < tone - 20.0,
                "noise lit {pos:?} at {got:.2} dB, within 20 dB of a tone's {tone:.2}",
            );
        }
    }

    /// A full-scale sine on a node folds to the power the analyzer reads it at,
    /// which is what "loud means the same across panes" rests on: the fold
    /// answers in the analyzer's own units, so the shared loudness curve maps
    /// it the way it maps the Spectral pane's own buckets.
    ///
    /// At C4, where a bin is 38¢ and the tone's main lobe is comfortably wider
    /// than the kernel that reads it. Higher up the two cross over and the mean
    /// starts averaging a narrow lobe against the quiet either side of it, which
    /// is the cost the module docs name — a full-scale tone at C7 folds to
    /// −5.8 dB. A gain rather than an error, and one the level window absorbs.
    #[test]
    fn a_full_scale_sine_folds_to_zero_db() {
        // On the root node's own pitch class, so nothing about the wheel's
        // placement is in the answer.
        let got = Bench::on(&sine(60.0)).db(NARROW, LatticePos::new(0, 0, 0));
        assert!(got.abs() < 2.0, "a full-scale sine folded to {got:.2} dB, not 0");
    }

    /// A partial off a node DIMS rather than switching off — the whole of why
    /// the width is a kernel and not a tolerance. Detune is a level, so ±15¢ of
    /// vibrato breathes where a gate would flicker.
    ///
    /// Swept at C6, which is where the KERNEL is what answers: lower down the
    /// analyzer's own bin is wider than the whole sweep and the curve is its
    /// rather than the fold's (see
    /// [`a_node_the_analyzer_resolves_apart_stays_dark`]).
    #[test]
    fn a_partial_off_a_node_dims_rather_than_switching_off() {
        let mut previous = f32::INFINITY;
        for cents in [0.0f32, 10.0, 20.0, 30.0] {
            let got = Bench::on(&sine(84.0 + cents / 100.0)).db(NARROW, LatticePos::new(0, 0, 0));
            assert!(got < previous, "{cents}¢ off read {got:.2} dB, no dimmer than {previous:.2}");
            // Still THERE at two standard deviations, where a Gaussian is a
            // seventh of its peak: dim is the claim, not gone.
            if cents <= 20.0 {
                assert!(got > -30.0, "{cents}¢ off switched the node off at {got:.2} dB");
            }
            previous = got;
        }
    }

    /// A width no bar can produce — a hand-edited blob's 0, a NaN — folds to
    /// finite numbers rather than dividing by zero and drawing a lattice that
    /// is silently dark.
    #[test]
    fn a_width_no_bar_can_produce_still_folds() {
        let bench = Bench::on(&sine(60.0));
        for width in [0.0, -3.0, f32::NAN, f32::INFINITY, 1e9] {
            let light = bench.light(width, LatticePos::new(0, 0, 0));
            assert!(light.total.is_finite(), "width {width} folded to {}", light.total);
            assert!(light.pitch.is_finite(), "width {width} put the pitch at {}", light.pitch);
            assert!(
                light.octaves.iter().all(|p| p.is_finite()),
                "width {width} poisoned the wedges: {:?}",
                light.octaves,
            );
        }
    }

    /// The toggle REPLACES. Held keys light nothing once the source is audio,
    /// and audio lights nodes no key is down for — the two halves of "never a
    /// blend", each of which a version that merely ADDED the fold would pass
    /// half of.
    #[test]
    fn the_audio_source_replaces_the_keys() {
        let scene = |state: &SharedState, spectral: bool| {
            let mut scene = derive_scene(
                &state.tracker,
                &state.tuning,
                &state.view,
                &state.frame_params,
                state.camera,
                None,
                1.0,
            );
            if spectral {
                light_from_spectrum(&mut scene, state, 1.0);
            }
            scene
        };
        let lit = |scene: &Scene| scene.nodes.iter().filter(|n| n.activation > 0.0).count();

        // A key down and nothing sounding.
        let mut state = fresh();
        state.frame_params.fade_time = 0.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        assert!(lit(&scene(&state, false)) > 0, "the keys light the lattice");
        assert_eq!(
            lit(&scene(&state, true)),
            0,
            "a held key lit the lattice with the source set to audio",
        );

        // ...and audio with no key down, which is the same claim from the other
        // end: with nothing held there is nothing for a blend to be made of.
        let mut state = fresh();
        state.spectrum_config.smoothing = 0.0;
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&sawtooth(48.0), 1, SR, 1.0, &cfg);
        assert_eq!(lit(&scene(&state, false)), 0, "nothing is held, so MIDI lights nothing");
        assert!(lit(&scene(&state, true)) > 0, "audio lit no node at all");
    }

    /// Only the LIGHTING is replaced: the geometry, the wheel, the grid and the
    /// melody/bass marks are `derive_scene`'s answers untouched, which is what
    /// makes this a post-pass rather than a second way of deriving a scene.
    #[test]
    fn the_source_leaves_everything_but_the_light_alone() {
        let mut state = fresh();
        state.frame_params.fade_time = 0.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&sawtooth(48.0), 1, SR, 1.0, &cfg);
        // Derived twice rather than cloned: a `Scene` is not `Clone`, and one
        // derivation is deterministic in its inputs, which the whole offline
        // renderer already rests on.
        let derive = || {
            derive_scene(
                &state.tracker,
                &state.tuning,
                &state.view,
                &state.frame_params,
                state.camera,
                None,
                1.0,
            )
        };
        let derived = derive();
        let mut relit = derive();
        light_from_spectrum(&mut relit, &state, 1.0);

        assert_eq!(relit.grid.len(), derived.grid.len(), "the grid was rebuilt");
        assert_eq!(relit.octave_layout, derived.octave_layout, "the wheel moved");
        for (was, now) in derived.nodes.iter().zip(&relit.nodes) {
            assert_eq!(now.lattice_pos, was.lattice_pos);
            assert_eq!(now.world_pos, was.world_pos, "{:?} moved", was.lattice_pos);
            assert_eq!(now.scale, was.scale, "{:?} changed size", was.lattice_pos);
            assert_eq!(now.cents, was.cents, "{:?} changed pitch class", was.lattice_pos);
            let at = was.lattice_pos;
            assert_eq!(now.melody_slots, was.melody_slots, "{at:?} lost its melody mark");
            assert_eq!(now.bass_slots, was.bass_slots, "{:?} lost its bass mark", was.lattice_pos);
            assert_eq!(now.trail, was.trail, "{:?} lost its trail", was.lattice_pos);
        }
        // ...and the light really did change, or the sweep above proves nothing.
        assert!(
            derived.nodes.iter().zip(&relit.nodes).any(|(a, b)| a.activation != b.activation),
            "no node's lighting changed at all",
        );
    }
}
