//! Measuring the analyzer's spectrum for the lattice's audio ring: how much
//! power is sounding at exactly the pitch each wedge of it names.
//!
//! ONE grid for the whole lattice, not a reading per node. A wedge names an
//! octave of its node's pitch class, so what it needs is a lookup at a pitch —
//! and the shader does that lookup itself, off a table this pass fills once.
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
//! ## Two readings, and only one of them folds
//!
//! [`apply`] fills the lattice's one audio channel — the ring inside the
//! octave band — with whichever of two readings
//! ([`ViewConfig::spectral_reading`](harmonigraph_scene::ViewConfig)) is
//! asked for, and they are measured differently on purpose:
//!
//! - **The fold** ([`SpectralReading::Fold`](harmonigraph_scene::SpectralReading))
//!   is what everything below describes: a kernel over the grid and a noise
//!   floor under it. It answers "is this pitch class sounding", which is a
//!   question about a NODE, and the shader reads one value of it per wedge.
//! - **The spectrum** ([`Spectrum`](harmonigraph_scene::SpectralReading::Spectrum))
//!   folds nothing at all. It hands the analyzer's grid to the shader whole and
//!   each wedge shows a window of it, so it answers "what is sounding NEAR this
//!   pitch, and how far off" — a question about a stretch of SPECTRUM, which
//!   every smoothing that helps the fold spoils.
//!
//! Either goes into one [`SpectralPaint`], which also carries the FREQUENCY
//! colour scheme — the analyzer's own ramp, its silent end pinned in chroma as
//! well as lightness onto the lattice's own ground
//! ([`ViewConfig::lattice_ground`](harmonigraph_scene::ViewConfig)) — so that
//! what the ring paints is the light the spectrogram, the spectrum curve and
//! the Spiral pane would paint it, and never the pitch ramp the MIDI picture
//! wears.
//!
//! Nothing here relights a NODE. The keys keep everything they draw, so a node
//! carries both pictures at once and neither has to be given up to see the
//! other.
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
//! The audio ring's Gate is a threshold in the same pass and is not an
//! exception to any of that. What everything above MEASURES is a weight to the
//! last byte, and the gate reads those values without changing one of them;
//! what it decides is whether a node's ring is on the screen at all
//! ([`Scene::wear_audio_rings`](harmonigraph_scene::Scene), run at the end of
//! [`apply`]). A wedge still dims rather than switching off — it is the whole
//! RING that comes and goes, at a level the view names, because the ring costs
//! its annulus at every node in the window whatever it reads. It comes and goes
//! on the note Fade, and a node the keys are holding keeps its ring whatever
//! the gate reads there, so the threshold never puts a step in the picture.
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
//! ## The reading has its own envelope
//!
//! [`AudioSpectrum::display`](crate::AudioSpectrum::display) arrives already
//! smoothed on the hop grid by the Analyzer's own Attack and Release, and the
//! ring then carries a SECOND envelope of its own ([`RingLevels`], on
//! `ViewConfig::spectral_ring_attack` / `_release`).
//!
//! Two speeds for one analyzer, deliberately, because the two pictures are
//! asked different questions. The Spectral pane is a measurement instrument and
//! wants what is there; the ring is a legibility device spread over hundreds of
//! nodes and wants whether a harmonic is PRESENT. A filter long enough to
//! settle the second is longer than the first should ever be, so one bar
//! settling both settles it wrong for one of them.
//!
//! It is stepped against the CLOCK rather than per call, exactly as the gate's
//! fade below is, so the two lattices an editor frame draws step it once and a
//! 60 fps render and a 144 Hz pane walk one curve. The state lives in
//! [`SharedState`], which is the whole of what the offline renderer carries
//! between frames.
//!
//! The GATE's answer does carry across frames
//! ([`RingFade`](harmonigraph_scene::RingFade)), and it is a different thing
//! being faded: not what a wedge reads but whether the node draws its ring at
//! all, which is a decision the Fade softens exactly as it softens a note's
//! own arrival and departure. That state lives in [`SharedState`], which is the
//! whole of what the offline renderer carries between frames, and it is stepped
//! against the CLOCK rather than per call — so the lattice being drawn TWICE in
//! an editor frame (the docked pane and the Render preview's copy) steps it
//! once, and a render at 60 fps and a pane at 144 walk the same transition.

use harmonigraph_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_BINS};
// The axis' own ends, read by `Fold::slot_power` — which is the shader's
// per-wedge read written out on the CPU, and so test-only along with them.
#[cfg(test)]
use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};
use harmonigraph_scene::{
    bucket_pitch, Scene, SpectralPaint, SpectralReading, ViewConfig, SPECTRAL_WIDTH_MAX,
    SPECTRAL_WIDTH_MIN,
};

use super::spectral::axes::{loudness, power_db, spectrogram_level_db};
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

/// The frame's fold: the kernel-weighted mean excess power at every bucket of
/// the analyzer's grid, which a wedge of the ring then reads at its own
/// octave's pitch.
///
/// **The whole fold is done once per lattice draw, not once per node**, and
/// that is what bounds its cost. The kernel is the same shape wherever it is
/// centered, so folding it over the grid ONCE answers every node and every
/// octave slot at a stroke — the alternative walks its own window per node per
/// slot, which at a deep window (1365 nodes, eleven octaves) is 1.5 million
/// bucket reads and was measured at 2.7 ms a frame, twice over in an editor
/// frame that draws the lattice for the pane and again for the Render preview.
/// This is 3828 buckets whatever the extents are, and the per-wedge work is one
/// interpolated read on the GPU.
///
/// What it costs is a fraction of a bucket's worth of blur: an octave's pitch
/// falls BETWEEN buckets, and where a per-node fold would centre its kernel
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

    /// The folded grid, bucket for bucket — what the ring's wedges are read
    /// off, in the analyzer's own power units.
    ///
    /// The same SHAPE as the analyzer's own grid rather than a table of its
    /// own, which is what lets one path carry both readings from here on: the
    /// ring is a window onto a grid of power at pitch, and whether that power
    /// was folded first is a question nothing past [`RingLevels::fill`] asks.
    pub(crate) fn grid(&self) -> &[f32; SPECTRUM_BINS] {
        &self.smoothed
    }

    /// The kernel-weighted mean power above the floor at absolute MIDI `pitch`,
    /// or `None` where the analyzer's axis does not reach that pitch.
    ///
    /// The CPU's form of what the shader does per wedge — `spectrum_at` on the
    /// grid above, at the octave's own pitch — and so test-only: it exists to
    /// let a claim about the fold be made in the dB the harmonic series
    /// predicts, rather than through a byte, a colour ramp and a GPU.
    #[cfg(test)]
    fn slot_power(&self, pitch: f32) -> Option<f32> {
        if !(SPECTRUM_MIN_MIDI..=SPECTRUM_MAX_MIDI).contains(&pitch) {
            return None;
        }
        // Bucket `b` stands for `SPECTRUM_MIN_MIDI + (b + 0.5) / 32` — the
        // grid's own convention, and the shader's `spectrum_at` subtracts the
        // same half so the fold and the ring place a partial at one pitch.
        let x = (pitch - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32 - 0.5;
        let i = (x.max(0.0) as usize).min(SPECTRUM_BINS - 2);
        let t = (x - i as f32).clamp(0.0, 1.0);
        Some(self.smoothed[i] + (self.smoothed[i + 1] - self.smoothed[i]) * t)
    }
}

/// Fill `scene`'s audio ring with whichever reading the view asks for
/// ([`ViewConfig::spectral_reading`](harmonigraph_scene::ViewConfig)).
///
/// One entry point and one output — the grid the ring's wedges are windows
/// onto — and the two readings differ only in what is measured into it:
///
/// - [`Fold`](harmonigraph_scene::SpectralReading::Fold) is the kernel over
///   the grid and the noise floor under it, everything above this line. It
///   answers "is this pitch class sounding", which is a question about a NODE,
///   and the shader reads one value of it per wedge.
/// - [`Spectrum`](harmonigraph_scene::SpectralReading::Spectrum) folds nothing
///   at all — the grid through the analyzer's Level window and no more —
///   because it answers "what is sounding NEAR this pitch, and how far off",
///   which is a question about a stretch of SPECTRUM and is spoiled by every
///   smoothing that helps the other.
///
/// The reading goes into the scene as one [`SpectralPaint`], which is also
/// what carries the FREQUENCY colour scheme: the volume gradient and range,
/// handed in whole here and baked into the ring's table by
/// [`SpectralPaint::new`], whose silent end is pinned — in chroma as well as
/// lightness — onto the lattice's own ground
/// ([`ViewConfig::lattice_ground`](harmonigraph_scene::ViewConfig)), so that
/// what the ring paints is the light the spectrogram, the spectrum curve and
/// the Spiral pane would paint it, opening on the grey a node's own rings rest
/// in rather than on the analyzer's black.
///
/// Nothing here touches a NODE. The MIDI picture is `derive_scene`'s answer
/// untouched — the bodies, the octave band, the marks, the trail, the camera —
/// and the measurement is a ring of its own inside the band, so one node
/// carries both readings and neither can be mistaken for the other.
///
/// With the ring's width at 0 — the LAYER's off position, and the only one it
/// has — nothing here runs at all, not even the read of
/// [`AudioSpectrum::display`](crate::AudioSpectrum::display), so the picture is
/// exactly the picture with this pass absent. The GEOMETRY is what is tested
/// rather than the reading beside it: a ring nobody can see is a measurement
/// nobody asked for, whichever of the two it would have carried. (The Gate
/// below is a different question and runs after the measuring rather than
/// instead of it: it holds back the NODES whose wedges say nothing, and needs
/// the reading to know which those are.)
pub(crate) fn apply(scene: &mut Scene, state: &mut SharedState, now: f64) {
    if !state.view.spectral_ring_draws() {
        return;
    }
    let reading = state.view.spectral_reading;
    let cfg = state.spectrum_config;
    let mut paint = SpectralPaint::new(&state.view, cfg.spectrogram_gradient);
    // The kernel is the FOLD's; `Spectrum` hands the analyzer's own grid
    // through untouched. A `bool` and not a `match` over two arms that
    // would read as a table of readings, where what this settles is one
    // question about one of them.
    let levels = state.spectrum.display(now);
    let folded = levels
        .filter(|_| reading == SpectralReading::Fold)
        .map(|levels| Fold::measure(levels, state.view.spectral_width));
    // With no audio flowing the target is silence rather than nothing at all,
    // so the reading LEAVES on its own release instead of dropping to the
    // ramp's floor in one frame the moment the analyzer stops answering.
    let grid = folded.as_ref().map(Fold::grid).or(levels);
    state.ring_levels.fill(&mut paint, &cfg, grid, &state.view, now);
    scene.spectral = paint;
    // Which nodes the ring is worth drawing on, now that there is something to
    // ask it of. Last, and off the scene rather than off the paint above,
    // because the answer is measured against the levels a wedge will actually
    // paint — see `Scene::wear_audio_rings`. With no audio flowing the grid is
    // zeros and every node is held back at any gate above its floor, which is
    // the point: an analyzer with nothing to say draws no rings rather than a
    // lattice of them at the ramp's floor.
    //
    // The note envelope, the same one every layer of a node runs on: the ring
    // is one of those layers, so it arrives and leaves on the Fade rather than
    // on a duration of its own. Assembled through `ViewConfig::envelope`, which
    // is the one place the Fade param and the Fade curve are put back together.
    let env = state.view.envelope(&state.frame_params);
    scene.wear_audio_rings(&mut state.ring_fade, &env, now);
}

/// What the ring's wedges READ, carried across frames on the ring's own attack
/// and release — where [`RingFade`](harmonigraph_scene::RingFade) is whether the
/// annulus is drawn at all.
///
/// Two envelopes and not one, because they answer different questions and a
/// single one would have to answer both wrong: the fade is a NODE's layer
/// arriving and leaving, which every other layer does on the note Fade, and
/// this is how fast a measurement inside that layer moves, which has nothing to
/// do with a note.
#[derive(Clone)]
pub struct RingLevels {
    /// What each bucket of the ring currently reads, 0..=1 on the analyzer's
    /// Level window — the envelope's state, kept in `f32` because the byte the
    /// shader gets is a quantization of it and stepping the byte instead would
    /// stall: a release slow enough to be worth having moves a bucket less than
    /// 1/255 in a frame, and rounding that back is a filter that never arrives.
    level: Box<[f32; SPECTRUM_BINS]>,
    /// The same reading carried through the volume-color window. Kept beside
    /// the analyzer level because values outside that window still need their
    /// true color position when the two ranges differ.
    color_level: Box<[f32; SPECTRUM_BINS]>,
    /// The clock the levels stand at, or `None` before the first fill.
    ///
    /// A moment and not a duration, so a pane drawn TWICE in one frame — the
    /// docked lattice and the Video tab's preview, off one clock — steps once:
    /// the second call's `dt` is zero and every bucket holds.
    at: Option<f64>,
}

impl Default for RingLevels {
    fn default() -> RingLevels {
        RingLevels {
            level: Box::new([0.0; SPECTRUM_BINS]),
            color_level: Box::new([0.0; SPECTRUM_BINS]),
            at: None,
        }
    }
}

impl RingLevels {
    /// Read a grid of power into the ring's channel, carried on the ring's own
    /// attack and release: every bucket through the analyzer's [`loudness`]
    /// curve and the independent volume-color mapping, stepped toward each
    /// target and quantized to the bytes the shader unpacks. The analyzer copy
    /// feeds the gate; the color copy keeps levels outside the analyzer window
    /// visible at their true position in the volume-color ramp.
    ///
    /// Per BUCKET rather than per node, which is what makes the ring cost the
    /// same whatever the extents are: the grid is one reading the whole lattice
    /// shares, and each node's wedges are a window onto it (the shader's
    /// `spectrum_at`). That holds for the fold as much as for the raw spectrum —
    /// the kernel is the same shape wherever it is centred, so folding it over
    /// the grid once answers every node and every octave at a stroke.
    ///
    /// `None` for the grid is the analyzer having nothing to say, which is a
    /// target of silence rather than an early return: the ring's own release
    /// carries it down, so the reading leaves the way it arrived.
    ///
    /// The FIRST fill of all settles rather than fading in — there is no
    /// transition to draw when nothing was on screen to transition from.
    fn fill(
        &mut self,
        paint: &mut SpectralPaint,
        cfg: &crate::SpectrumConfig,
        grid: Option<&SpectrumBuckets>,
        view: &ViewConfig,
        now: f64,
    ) {
        let dt = self.at.map(|at| now - at);
        self.at = Some(now);
        // No clock behind it: take the reading outright, which is what makes
        // the first frame the analyzer's picture and not a fade of one.
        let (attack, release) = match dt {
            Some(dt) => (
                crate::spectrum::hop_alpha(view.spectral_ring_attack, dt),
                crate::spectrum::hop_alpha(view.spectral_ring_release, dt),
            ),
            None => (1.0, 1.0),
        };
        for (bucket, level) in self.level.iter_mut().enumerate() {
            // The bucket's own centre pitch, because the tilt in `loudness` is
            // a function of pitch and because the shader reads this table back
            // on the same convention.
            let target = grid
                .map_or(0.0, |grid| loudness(cfg, grid[bucket], bucket_pitch(bucket)))
                .clamp(0.0, 1.0);
            let alpha = if target > *level { attack } else { release };
            *level += (target - *level) * alpha;
            paint.levels[bucket] = (*level * 255.0).round() as u8;

            let color_target = grid.map_or(0.0, |grid| {
                spectrogram_level_db(cfg, power_db(grid[bucket]), bucket_pitch(bucket))
            });
            let color_level = &mut self.color_level[bucket];
            let color_alpha = if color_target > *color_level { attack } else { release };
            *color_level += (color_target - *color_level) * color_alpha;
            paint.color_levels[bucket] = (*color_level * 255.0).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::fresh;
    use harmonigraph_core::{spectrum::midi_to_hz, LatticePos, NoteEvent, Tuning};
    use harmonigraph_scene::{
        derive_scene, octave_layout, OctaveLayout, ViewConfig, MAX_SPAN, OCTAVE_SLOTS,
    };

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

    /// One node's ring as the shader reads it: the folded power at each drawn
    /// wedge's own octave pitch.
    ///
    /// The CPU's form of `spectral_ring`'s fold branch, and the reason the
    /// claims below can be stated in dB: what reaches the picture is this
    /// through [`loudness`], a byte and a colour ramp, none of which the
    /// harmonic series has anything to say about.
    struct NodeRing {
        /// Power at each drawn slot; slots the wheel does not draw are 0,
        /// exactly as they are unpainted.
        octaves: [f32; OCTAVE_SLOTS],
        /// Their sum — what a reader integrates across a node's whole ring,
        /// and where the constellation states itself.
        total: f32,
    }

    /// One signal analyzed, ready to read rings out of.
    struct Bench {
        levels: SpectrumBuckets,
        layout: OctaveLayout,
        tuning: Tuning,
    }

    impl Bench {
        /// Analyze `samples` on a wheel drawing EVERY octave slot, in JUST
        /// intonation — which is the material this panel is aimed at, and the
        /// whole reason the constellation is exact: a partial of a just-tuned
        /// note lands ON its node rather than near it.
        ///
        /// The widest wheel rather than the fresh five slices, because these
        /// tests are about the fold and not about the wheel: a narrow wheel
        /// would leave "the octaves below every partial stay dark" true of
        /// slots that are not drawn at all, which is no claim about the
        /// measurement. Where the wheel is set is `octave_layout`'s own
        /// question and its own tests.
        fn on(samples: &[f32]) -> Bench {
            let mut state = fresh();
            // The display's EMA is the Analyzer's Smoothing control, and it is
            // an inheritance the fold is glad of in the plugin and cannot use
            // here: it would make every number below a function of how many
            // hops the fixture happened to push.
            state.spectrum_config.attack = 0.0;
            state.spectrum_config.release = 0.0;
            let cfg = state.spectrum_config;
            state.spectrum.push_samples(samples, 1, SR, 1.0, &cfg);
            let levels = *state.spectrum.display(1.0).expect("a second of audio is enough");
            let view = ViewConfig::default();
            Bench {
                levels,
                layout: octave_layout(
                    MAX_SPAN,
                    view.octave_center,
                    0,
                    view.octave_extra_size,
                    view.octave_extra_blend,
                ),
                tuning: Tuning::just(),
            }
        }

        fn light(&self, width: f32, pos: LatticePos) -> NodeRing {
            self.at_class(width, self.tuning.pitch_class(pos).to_cents())
        }

        /// The same for a pitch class named directly, which is what the
        /// selectivity tests want: they ask how far apart two classes have to
        /// be, and a lattice position is a roundabout way of naming one.
        fn at_class(&self, width: f32, cents: f32) -> NodeRing {
            let fold = Fold::measure(&self.levels, width);
            let (low, high) = self.layout.slots(cents);
            let mut octaves = [0.0f32; OCTAVE_SLOTS];
            for slot in low..=high {
                let Ok(drawn) = usize::try_from(slot) else { continue };
                if drawn >= OCTAVE_SLOTS {
                    continue;
                }
                // At the slot's OWN pitch and nothing folded onto it from
                // outside the wheel: a wedge names one octave and reads there,
                // which is the whole of what makes it one number.
                octaves[drawn] =
                    fold.slot_power(self.layout.slot_pitch(slot, cents)).unwrap_or(0.0);
            }
            NodeRing { total: octaves.iter().sum(), octaves }
        }

        /// The whole ring's power in dB, which is where the harmonic series
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

    /// Noise does not light the rings. Broadband sound has no partials, so
    /// what the floor subtraction leaves is what did not stand above its own
    /// neighbourhood — which for white noise is a whisper.
    ///
    /// Against a TONE rather than against an absolute level, because an
    /// absolute one is a claim about how loud the fixture's noise happens to
    /// be: what has to hold is that a pitch and a hiss of the same power are
    /// nowhere near each other on the lattice.
    #[test]
    fn noise_alone_does_not_light_the_rings() {
        let tone = Bench::on(&sine(60.0)).db(NARROW, LatticePos::new(0, 0, 0));
        let hiss = Bench::on(&noise());
        // The whole fresh window, and the LOUDEST node in it: a floor estimate
        // is a statistic and 273 nodes is 273 draws from it, so what a haze
        // looks like on screen is set by its luckiest node rather than by its
        // average one. Full-scale noise leaves that node 26 dB under a
        // full-scale tone, which is a whisper against the default 60 dB window
        // and is the "dim floor" the design admits to rather than nothing at
        // all — the honest answer, since those frequencies really are sounding.
        for pos in ViewConfig::default().reach().positions() {
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
    /// finite numbers rather than dividing by zero and drawing a ring that is
    /// silently dark.
    ///
    /// The whole grid and not one node's wedges, because the divisor is the
    /// kernel's and the kernel is the grid's: a NaN there poisons every bucket
    /// at once, and a byte cast from one is 0 with nothing on screen saying so.
    #[test]
    fn a_width_no_bar_can_produce_still_folds() {
        let bench = Bench::on(&sine(60.0));
        for width in [0.0, -3.0, f32::NAN, f32::INFINITY, 1e9] {
            let fold = Fold::measure(&bench.levels, width);
            assert!(
                fold.grid().iter().all(|p| p.is_finite()),
                "width {width} poisoned the grid the ring reads",
            );
            let light = bench.light(width, LatticePos::new(0, 0, 0));
            assert!(light.total.is_finite(), "width {width} folded to {}", light.total);
            assert!(
                light.octaves.iter().all(|p| p.is_finite()),
                "width {width} poisoned the wedges: {:?}",
                light.octaves,
            );
        }
    }

    /// A scene derived exactly as the Lattice pane derives it, then handed to
    /// this pass — which reads the view's own selector for what to fill the
    /// ring with, so a test says which reading it is asking for by setting it.
    ///
    /// At ONE clock, so a state handed here twice draws the second scene's
    /// rings where the first settled them (`RingFade` steps against the clock).
    /// Every claim below but one is about a single frame; the one that is not
    /// asks for its own moments through [`scene_of_at`].
    fn scene_of(state: &mut SharedState) -> Scene {
        scene_of_at(state, 1.0)
    }

    /// [`scene_of`] at a stated clock, for the claims about how a ring comes
    /// and goes rather than about what it reads. A state carried across two of
    /// these has a fade running through it, exactly as a shell does.
    fn scene_of_at(state: &mut SharedState, now: f64) -> Scene {
        let mut scene = derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.view.reach(),
            &state.frame_params,
            state.camera,
            None,
            now,
        );
        apply(&mut scene, state, now);
        scene
    }

    /// EITHER reading adds. The ring fills a channel of its own — one reading
    /// of the spectrum the whole lattice shares — and leaves every MIDI answer
    /// exactly as `derive_scene` wrote it, so one node carries both pictures at
    /// once and neither has to be given up to see the other.
    ///
    /// A per-node sweep because a pass that reached into the nodes AT ALL is
    /// the way this would stop being true, and both readings because the fold
    /// is the one with a per-node answer to be tempted by: a fold written into
    /// the bodies and the band would draw a plausible picture that has thrown
    /// the player's part away.
    ///
    /// A held C4 with a C3 saw sounding, so the two channels have something to
    /// disagree about: the keys light one pitch class, the saw sounds at many.
    #[test]
    fn either_reading_draws_beside_the_keys_rather_than_instead_of_them() {
        let mut state = fresh();
        state.frame_params.fade_time = 0.0;
        state.spectrum_config.attack = 0.0;
        state.spectrum_config.release = 0.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&sawtooth(48.0), 1, SR, 1.0, &cfg);

        // The MIDI picture alone, taken with the ring dialled to no width —
        // its one off switch, and the state every comparison below is against.
        let ring_width = state.view.spectral_ring_width;
        state.view.spectral_ring_width = 0.0;
        let midi = scene_of(&mut state);
        assert!(!midi.spectral.ring_draws(), "the ring drew with no width to draw in");
        state.view.spectral_ring_width = ring_width;
        assert!(
            midi.nodes.iter().any(|n| n.activation > 0.0),
            "the keys lit nothing, so there is no MIDI picture to leave alone",
        );

        let mut grids = Vec::new();
        for reading in [SpectralReading::Fold, SpectralReading::Spectrum] {
            state.view.spectral_reading = reading;
            // From a standing start, like `the_two_readings_are_measured_apart`
            // and for its reason: both scenes are derived at ONE clock, where
            // the ring's envelope holds rather than steps, so a table left
            // carrying the previous reading is handed straight back.
            state.ring_levels = RingLevels::default();
            let both = scene_of(&mut state);
            for (was, now) in midi.nodes.iter().zip(&both.nodes) {
                let at = was.lattice_pos;
                assert_eq!(now.activation, was.activation, "{reading:?}: {at:?} changed how lit");
                assert_eq!(now.octaves, was.octaves, "{reading:?}: {at:?} lost its held octaves");
                assert_eq!(now.color, was.color, "{reading:?}: {at:?} was repainted");
                assert_eq!(now.melody_slots, was.melody_slots, "{reading:?}: {at:?} lost its mark");
            }
            assert!(both.spectral.ring_draws(), "{reading:?} left the ring's annulus empty");
            // The saw really is in the grid the ring reads. A count of loud
            // buckets rather than a node count, because the reading is not per
            // node: the ring is a window onto this one table, and what a given
            // node shows of it is the shader's arithmetic (pinned on the GPU by
            // `the_audio_ring_reads_the_spectrum_around_each_octave`).
            let loud = both.spectral.levels.iter().filter(|&&level| level > 128).count();
            eprintln!("{reading:?}: {loud} of {SPECTRUM_BINS} buckets over half the Level window");
            assert!(loud > 0, "{reading:?} left nothing in the grid the ring reads");
            grids.push(both.spectral.levels.clone());
        }

        // And the two arms measured two readings. `loud > 0` above is a
        // liveness check on each grid and cannot tell one from the other, so
        // without this the Spectrum arm can hand back the Fold's grid and the
        // whole loop still passes — which is what the ring's CARRIED levels
        // do: both scenes are derived at one clock, where the envelope holds
        // rather than steps, so a reading changed between them is measured
        // into a table that already holds the last one's answer.
        //
        // That the two readings are far apart is
        // `the_two_readings_are_measured_apart`'s subject, at over a thousand
        // buckets of this same grid; here it is only asked that they are not
        // the SAME table twice.
        let differing =
            grids[0].iter().zip(grids[1].iter()).filter(|(a, b)| a.abs_diff(**b) > 1).count();
        assert!(
            differing > 0,
            "both readings came back as one grid, so one of the two arms measured the other's",
        );
    }

    /// The ring reads the analyzer RAW: bucket for bucket, its levels are the
    /// [`loudness`] curve applied to the analyzer's own grid, with no kernel
    /// over it and no floor under it.
    ///
    /// The claim that keeps the ring's geometry on the analyzer's scale, and it
    /// is exact rather than approximate: the byte in the table is what
    /// `loudness` answers. A fold or a floor slipped in here would leave the ring
    /// reading lower than the analyzer everywhere, which looks like a taste
    /// decision rather than a bug.
    #[test]
    fn the_rings_levels_are_the_analyzers_own() {
        let mut state = fresh();
        state.spectrum_config.attack = 0.0;
        state.spectrum_config.release = 0.0;
        state.view.spectral_reading = SpectralReading::Spectrum;
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&sawtooth(48.0), 1, SR, 1.0, &cfg);

        let scene = scene_of(&mut state);
        let grid = state.spectrum.display(1.0).expect("a second of audio is enough");
        let mut checked = 0;
        for (bucket, &power) in grid.iter().enumerate() {
            let want = loudness(&cfg, power, bucket_pitch(bucket));
            let got = f32::from(scene.spectral.levels[bucket]) / 255.0;
            assert!(
                (got - want.clamp(0.0, 1.0)).abs() <= 0.5 / 255.0,
                "bucket {bucket} reads {got} where the analyzer reads {want}",
            );
            if want > 0.0 {
                checked += 1;
            }
        }
        assert!(checked > 0, "no bucket was above the floor, so the comparison proves nothing");
    }

    /// The ring keeps analyzer levels for its gate but maps colors from the
    /// independent volume window, including values below the analyzer floor.
    #[test]
    fn the_ring_color_levels_keep_values_outside_the_analyzer_window() {
        let mut cfg = crate::SpectrumConfig {
            floor_db: -60.0,
            ceiling_db: -20.0,
            volume_floor_db: -90.0,
            volume_ceiling_db: -30.0,
            tilt: 0.0,
            ..crate::SpectrumConfig::default()
        };
        cfg.sanitize();
        let view = ViewConfig::default();
        let bucket = 1000;
        let power = 10.0f32.powf(-7.5); // -75 dB, below the analyzer floor
        let mut grid = [0.0f32; SPECTRUM_BINS];
        grid[bucket] = power;
        let mut paint = SpectralPaint::new(&view, cfg.spectrogram_gradient);
        RingLevels::default().fill(&mut paint, &cfg, Some(&grid), &view, 0.0);

        let analyzer = loudness(&cfg, power, bucket_pitch(bucket));
        let color = spectrogram_level_db(&cfg, power_db(power), bucket_pitch(bucket));
        assert_eq!(analyzer, 0.0, "the fixture must sit below the analyzer floor");
        assert!(color > 0.0, "the color window must retain the below-floor value");
        assert_eq!(paint.levels[bucket], 0, "the gate level left the analyzer scale");
        assert_eq!(
            paint.color_levels[bucket],
            (color * 255.0).round() as u8,
            "the ring color level did not use the independent window",
        );
    }

    /// The two readings are MEASURED apart, which is the whole point of having
    /// a choice: one is a kernel over a local noise floor and the other is the
    /// analyzer's grid untouched, so between a saw's partials the raw reading
    /// stands well above what the fold answers at the same pitch.
    ///
    /// The claim that keeps the selector from being one picture with two
    /// labels. A change that quietly routed either through the other's
    /// measurement — a floor slipped under the raw grid, a kernel dropped from
    /// the fold — would leave both readings drawing very nearly the same ring
    /// and neither test above would notice.
    ///
    /// A sweep over the WHOLE grid rather than at chosen nodes, because that is
    /// what a node's wedges are windows onto: the readings differ or they do
    /// not, bucket for bucket, and no node has to be picked for it to be true.
    #[test]
    fn the_two_readings_are_measured_apart() {
        let mut state = fresh();
        state.spectrum_config.attack = 0.0;
        state.spectrum_config.release = 0.0;
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&sawtooth(48.0), 1, SR, 1.0, &cfg);

        // Each reading measured from a standing start. Both scenes are derived
        // at one clock, where the ring's envelope holds rather than steps (a
        // pane drawn twice in a frame must not run it twice), so without the
        // reset the second reading would be handed back the first one's levels.
        state.view.spectral_reading = SpectralReading::Fold;
        state.ring_levels = RingLevels::default();
        let folded = scene_of(&mut state);
        state.view.spectral_reading = SpectralReading::Spectrum;
        state.ring_levels = RingLevels::default();
        let raw = scene_of(&mut state);
        assert!(folded.spectral.folded, "the fold is not read at its wedges' own pitches");
        assert!(!raw.spectral.folded, "the raw reading lost its window across the wedge");

        // The FLOOR is what separates them, and it separates them downward:
        // between a saw's partials the fold subtracts what did not stand above
        // its own neighbourhood, so it reads far under the analyzer's own
        // buckets there. Not everywhere, and the exception is the kernel rather
        // than a defect — a Gaussian mean spreads a partial onto its
        // neighbours, so a bucket in a partial's skirt can fold ABOVE its own
        // raw level. That is the fold admitting a detuned partial, which is
        // what the width bar is for.
        let (mut under, mut differing) = (0.0f32, 0usize);
        for (&fold, &grid) in folded.spectral.levels.iter().zip(raw.spectral.levels.iter()) {
            under = under.max(f32::from(grid.saturating_sub(fold)) / 255.0);
            // A byte's own step of slack: the two paths quantize separately.
            differing += usize::from(fold.abs_diff(grid) > 1);
        }
        eprintln!("{differing} buckets differ; the raw grid reads up to {under:.3} over the fold");
        assert!(
            under > 0.05,
            "the fold never reads under the analyzer (largest gap {under:.3}), \
             so its noise floor is not coming off",
        );
        assert!(
            differing > SPECTRUM_BINS / 10,
            "only {differing} of {SPECTRUM_BINS} buckets differ, so the two readings \
             are very nearly one table and the choice between them says nothing",
        );
    }

    /// The plugin has exactly TWO colour schemes, and which one a thing wears
    /// is decided by what it MEASURES rather than by which pane it is on.
    ///
    /// - **MIDI**: `ViewConfig::pitch_gradient` indexed by a PITCH. The
    ///   lattice's nodes, wedges and marks, and the roll's ribbons.
    /// - **FREQUENCY**: `SpectrumConfig::spectrogram_gradient` indexed by a
    ///   LEVEL. The spectrum curve, the spectrogram's cells, the Spiral pane's
    ///   segments, and everything the lattice lights from audio. One LIGHT per
    ///   level, standing on whatever ground the surface it lands on has: the
    ///   heatmap's is black, the ring's is the lattice.
    ///
    /// Held against `ring_gradient` — the scene crate's own re-anchoring, the
    /// one the ring's table is actually built through — rather than against a
    /// copy of its arithmetic written out here. The claim is that the ring
    /// paints the volume gradient at all, and a restated formula would go
    /// on passing after the two definitions had drifted apart.
    ///
    /// Worth pinning because both halves have already drifted once and neither
    /// drift is visible as a bug: an audio reading painted off the pitch ramp
    /// reads as a plausible picture that means something else entirely, and a
    /// ribbon painted off a ramp of the roll's own reads as a note that is not
    /// the node it lit.
    #[test]
    fn the_plugin_has_two_colour_schemes_and_audio_wears_the_analyzers() {
        use crate::panes::spectral::roll::note_color;

        let mut state = fresh();
        state.view.spectral_reading = SpectralReading::Spectrum;
        state.spectrum_config.attack = 0.0;
        state.spectrum_config.release = 0.0;
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&sawtooth(48.0), 1, SR, 1.0, &cfg);
        let scene = scene_of(&mut state);

        // The ring's ramp is the heatmap's gradient, entry for entry, with its
        // silent end standing on the node's own ground instead of on the
        // heatmap's black plane. So a wedge at a level and a cell at that level
        // are one reading rather than two, and where the ramp opens is the
        // whole of the difference.
        let ring = harmonigraph_scene::pitch_ramp_lut(harmonigraph_scene::ring_gradient(
            cfg.spectrogram_gradient,
            state.view.lattice_ground_lightness(),
        ));
        for (k, entry) in scene.spectral.lut.iter().enumerate() {
            let got = crate::panes::scene_color(*entry, 1.0);
            let want = crate::panes::scene_color(ring[k], 1.0);
            assert_eq!(got, want, "entry {k} of the ring's ramp is not the heatmap's light");
        }
        // The top of the range is where the two tables MEET: the anchoring
        // moves the ramp's silent end and leaves the loud one exactly where
        // the analyzer put it, so a loud wedge and a loud cell are one colour
        // rather than two that nearly agree.
        let top = harmonigraph_scene::PITCH_LUT_N - 1;
        assert_eq!(
            crate::panes::scene_color(scene.spectral.lut[top], 1.0),
            crate::panes::scene_color(
                harmonigraph_scene::gradient_color(1.0, cfg.spectrogram_gradient),
                1.0,
            ),
            "the loudest wedge is not the colour the heatmap draws at that level",
        );
        // Every analyzer preset opens at black, because the heatmap's plane is
        // black; on the lattice that end stands on the node's own ground, the
        // same grey the octave band beside it draws where nothing sounds. A
        // silent wedge is drawn deliberately — a reading, not a gap — so this
        // is most of the ring most of the time, and unanchored it is a hole at
        // every node.
        //
        // Against `scene.lattice_ground` and not against a number: that field IS
        // what the shader lays down for the band, so this is the two layers
        // measured against each other through the whole path a frame takes.
        let floor = scene.spectral.lut[0];
        let step = (floor.truncate() - scene.lattice_ground.truncate()).abs().max_element();
        assert!(
            step * 255.0 < 0.5,
            "a silent wedge draws {floor:?} where the octave band draws {:?}",
            scene.lattice_ground,
        );

        // ...and the MIDI half: a ribbon and the node it lit are one colour off
        // one gradient, which is the claim `note_color` exists to keep.
        for pitch in [36.0f32, 60.0, 72.5, 96.0] {
            let node = harmonigraph_scene::pitch_lut_color(
                pitch,
                state.frame_params.darkest_pitch,
                state.frame_params.brightest_pitch,
                state.view.pitch_gradient,
            );
            assert_eq!(
                note_color(&state, pitch, 1.0),
                crate::panes::scene_color(node, 1.0),
                "a ribbon at MIDI {pitch} is not the colour the node at that pitch wears",
            );
        }
    }

    /// Before any audio has flowed the ring's ANNULUS is still real and its
    /// grid is still zero: the reading is there and it says "nothing sounds
    /// here", which is what a level of 0 means and not an absent layer.
    ///
    /// The geometry alone, deliberately, and the geometry is where the two
    /// answers part company: whether the ring layer is on is the width bar's
    /// question and is settled here, and whether a given NODE wears it with
    /// nothing sounding is the Gate's — at any setting above its floor, none of
    /// them do (`silence_rings_nothing_and_a_tone_rings_its_own_class`). A
    /// version that emptied the annulus instead would look the same on a silent
    /// lattice and would take the ring away for good.
    ///
    /// Both readings, since the fold has a whole measuring pass of its own that
    /// there is nothing to run.
    #[test]
    fn the_ring_draws_its_floor_before_any_audio_flows() {
        for reading in [SpectralReading::Fold, SpectralReading::Spectrum] {
            let mut state = fresh();
            state.view.spectral_reading = reading;
            let scene = scene_of(&mut state);
            assert!(
                scene.spectral.ring_draws(),
                "with no audio flowing {reading:?} vanished instead of wearing the floor",
            );
            assert!(
                scene.spectral.levels.iter().all(|&level| level == 0),
                "a grid nothing fed reads a level other than the floor under {reading:?}",
            );
        }
    }

    /// The gate reaches the picture from real audio: with nothing sounding no
    /// node rings, and a lone tone rings its OWN pitch class and leaves the
    /// lattice around it alone.
    ///
    /// The end-to-end claim, made where the analysis actually happens —
    /// `harmonigraph_scene`'s own tests measure the gate against a grid handed
    /// to it, and every one of them would go on passing if this pass forgot to
    /// call it, or called it before the levels were measured in.
    ///
    /// The silence half is the one the fresh setting was chosen for. Ungated,
    /// an analyzer with nothing to say draws a ring at the ramp's floor on
    /// every node in view — an honest reading, and hundreds of them saying only
    /// where the nodes are.
    #[test]
    fn silence_rings_nothing_and_a_tone_rings_its_own_class() {
        let quiet = scene_of(&mut fresh());
        assert!(quiet.spectral.ring_draws(), "the fresh ring is off, so nothing is being gated");
        assert!(quiet.spectral.gate > 0.0, "the fresh gate is at its floor");
        assert!(
            quiet.nodes.iter().all(|n| n.audio_ring == 0.0),
            "{} of {} nodes rang with no audio flowing",
            quiet.nodes.iter().filter(|n| n.audio_ring > 0.0).count(),
            quiet.nodes.len(),
        );

        let mut state = fresh();
        state.spectrum_config.attack = 0.0;
        state.spectrum_config.release = 0.0;
        let cfg = state.spectrum_config;
        // A full-scale sine at middle C: one partial, so what should ring is
        // the C nodes and nothing else. A saw would light its whole
        // constellation, which is the right picture and a poor test.
        state.spectrum.push_samples(&sine(60.0), 1, SR, 1.0, &cfg);
        let scene = scene_of(&mut state);
        let (mut rang, mut dark) = (0, 0);
        for node in &scene.nodes {
            let off = node.cents.min(1200.0 - node.cents);
            if node.audio_ring > 0.0 {
                rang += 1;
                assert!(
                    off < 30.0,
                    "{:?} at {}¢ rang with nothing sounding within 30¢ of it",
                    node.lattice_pos,
                    node.cents,
                );
            } else {
                dark += 1;
            }
        }
        eprintln!("a full-scale C rang {rang} of {} nodes", scene.nodes.len());
        assert!(rang > 0, "a full-scale tone rang no node at all");
        assert!(dark > rang, "a lone tone rang {rang} nodes and left only {dark} dark");
    }

    /// A ring arrives on the FADE, end to end: a tone reaching a lattice that
    /// has been quiet rings its class part way through the duration and fully
    /// once it has run out.
    ///
    /// The wiring is what this is for, and it is wiring nothing else here
    /// touches: the duration is a host param and the curve a view setting, put
    /// back together by `ViewConfig::envelope` — so a pass that reached for one
    /// of them alone, or built an envelope of its own, would draw a ring
    /// arriving at a speed no bar on screen names. Every claim in
    /// `harmonigraph_scene` about the fade would go on passing.
    #[test]
    fn a_ring_arrives_on_the_fade_the_notes_run_on() {
        let mut state = fresh();
        // A straight line of a stated length, so half way in is half way
        // along; the fresh curve is not, and this is not the test for it.
        state.frame_params.fade_time = 1.0;
        state.view.fade_shape = 0.0;
        state.spectrum_config.attack = 0.0;
        state.spectrum_config.release = 0.0;
        let cfg = state.spectrum_config;

        // A quiet frame first, so there is a picture to arrive FROM: the very
        // first step of all settles rather than fading in.
        let quiet = scene_of_at(&mut state, 1.0);
        assert!(
            quiet.nodes.iter().all(|n| n.audio_ring == 0.0),
            "the lattice was already ringing before the tone arrived",
        );

        // The C nodes are the ones a full-scale middle C opens; their level is
        // what the fade is read on.
        let rung = |scene: &Scene| {
            scene
                .nodes
                .iter()
                .filter(|n| n.cents.min(1200.0 - n.cents) < 5.0)
                .map(|n| n.audio_ring)
                .fold(0.0f32, f32::max)
        };
        state.spectrum.push_samples(&sine(60.0), 1, SR, 1.5, &cfg);
        let half = rung(&scene_of_at(&mut state, 1.5));
        assert!((half - 0.5).abs() < 1e-4, "half a Fade after the tone a C rings {half}");
        state.spectrum.push_samples(&sine(60.0), 1, SR, 2.5, &cfg);
        let full = rung(&scene_of_at(&mut state, 2.5));
        assert_eq!(full, 1.0, "a Fade after the tone a C rings {full}");
    }

    /// The gate's floor is the ungated picture end to end: every node rings,
    /// silence included, which is the whole reading at once and what the bar's
    /// off position has to give back through the real pass rather than only in
    /// `harmonigraph_scene`'s own units.
    #[test]
    fn the_gates_floor_rings_every_node() {
        let mut state = fresh();
        state.view.spectral_ring_gate = 0.0;
        let scene = scene_of(&mut state);
        assert!(
            scene.nodes.iter().all(|n| n.audio_ring == 1.0),
            "a gate at its floor held back {} nodes",
            scene.nodes.iter().filter(|n| n.audio_ring < 1.0).count(),
        );
    }

    /// A probe at a bucket's own centre reads that bucket, not a blend of it
    /// and its neighbour: the reader subtracts the grid's half-bucket offset
    /// exactly as `bucket_pitch` and the shader's `spectrum_at` do, so both
    /// readings place a partial at the same pitch.
    #[test]
    fn a_probe_at_a_buckets_centre_reads_that_bucket_alone() {
        let mut levels = [0.0f32; SPECTRUM_BINS];
        let bucket = 2000;
        levels[bucket] = 1.0;
        let fold = Fold::measure(&levels, SPECTRAL_WIDTH_MIN);

        // Tolerances are f32 grid arithmetic, not slack: recovering the index
        // from an absolute pitch wobbles by ~1e-4 of a bucket, and the levels
        // reach the GPU as bytes (a step of ~4e-3) anyway. The bug this pins
        // read the centre HALF a bucket off — three orders of magnitude out.
        let centre = harmonigraph_scene::bucket_pitch(bucket);
        let at_centre = fold.slot_power(centre).unwrap();
        assert!(
            (at_centre - fold.smoothed[bucket]).abs() < 1e-3,
            "the centre of bucket {bucket} reads {at_centre}, not its own smoothed value {}",
            fold.smoothed[bucket],
        );

        // And symmetrically off it: a quarter-bucket flat and a quarter-bucket
        // sharp of an isolated partial are the same distance from it, so they
        // read the same power.
        let quarter = 0.25 / BINS_PER_SEMITONE as f32;
        let flat = fold.slot_power(centre - quarter).unwrap();
        let sharp = fold.slot_power(centre + quarter).unwrap();
        assert!(
            (flat - sharp).abs() < 1e-3,
            "a quarter-bucket flat reads {flat} and a quarter-bucket sharp reads {sharp}",
        );
    }

    // ---- The ring's own ballistics ----------------------------------------

    /// A view with the ring drawn and stated times on it.
    fn timed(attack: f32, release: f32) -> ViewConfig {
        ViewConfig {
            spectral_ring_attack: attack,
            spectral_ring_release: release,
            ..ViewConfig::default()
        }
    }

    /// One bucket's byte after filling from a flat grid of `power`, stepping
    /// `steps` frames of `dt` each.
    fn carried(view: &ViewConfig, first: f32, then: f32, dt: f64, steps: usize) -> Vec<u8> {
        let cfg = crate::SpectrumConfig::default();
        let mut ring = RingLevels::default();
        let mut paint = SpectralPaint::new(view, Default::default());
        let flat = |p: f32| -> SpectrumBuckets { [p; SPECTRUM_BINS] };
        // The first fill settles, which is what makes the steps below a
        // transition from a known place rather than from a fade-in.
        ring.fill(&mut paint, &cfg, Some(&flat(first)), view, 0.0);
        let mut out = vec![paint.levels[1000]];
        for step in 1..=steps {
            ring.fill(&mut paint, &cfg, Some(&flat(then)), view, dt * step as f64);
            out.push(paint.levels[1000]);
        }
        out
    }

    /// The first fill of all settles: there is no transition to draw when
    /// nothing was on screen to transition from, so a lattice whose ring has
    /// just been switched on shows the analyzer rather than a fade-in.
    #[test]
    fn the_first_reading_settles_rather_than_fading_in() {
        let view = timed(1.0, 1.0);
        let quiet = carried(&view, 0.0, 0.0, 0.1, 0);
        let loud = carried(&view, 1.0, 1.0, 0.1, 0);
        assert_eq!(quiet[0], 0, "silence settles at the floor");
        assert!(loud[0] > 200, "a full-scale grid settles near the top, not part way up");
    }

    /// Up on the attack and down on the release, and the two are independent:
    /// the same step of the same size takes a different time each way.
    #[test]
    fn a_reading_rises_on_the_attack_and_falls_on_the_release() {
        // Fast up, slow down — the shape the ring ships with.
        let view = timed(0.01, 1.0);
        let up = carried(&view, 0.0, 1.0, 0.05, 1);
        let down = carried(&view, 1.0, 0.0, 0.05, 1);
        assert!(
            up[1] > 200,
            "a 50 ms step on a 10 ms attack should be nearly all the way up, not {}",
            up[1],
        );
        assert!(
            down[1] > 200,
            "the same step on a 1 s release should barely have moved, but it reads {}",
            down[1],
        );

        // And swapped, the picture swaps with it — so what is being measured is
        // the two times rather than a direction baked into the filter.
        let view = timed(1.0, 0.01);
        let up = carried(&view, 0.0, 1.0, 0.05, 1);
        let down = carried(&view, 1.0, 0.0, 0.05, 1);
        assert!(up[1] < 55, "a slow attack should barely have risen, but it reads {}", up[1]);
        assert!(down[1] < 55, "a fast release should be nearly down, but it reads {}", down[1]);
    }

    /// One frame drawn TWICE steps the envelope once — the docked lattice and
    /// the Video tab's preview come off one clock, and a reading that stepped
    /// per call would run at twice the speed whenever both are on screen.
    #[test]
    fn one_frame_drawn_twice_carries_the_reading_once() {
        let view = timed(0.1, 0.1);
        let cfg = crate::SpectrumConfig::default();
        let loud = [1.0f32; SPECTRUM_BINS];
        let mut once = RingLevels::default();
        let mut paint = SpectralPaint::new(&view, Default::default());
        once.fill(&mut paint, &cfg, Some(&[0.0; SPECTRUM_BINS]), &view, 0.0);
        once.fill(&mut paint, &cfg, Some(&loud), &view, 0.05);
        let stepped = paint.levels[1000];

        let mut twice = RingLevels::default();
        twice.fill(&mut paint, &cfg, Some(&[0.0; SPECTRUM_BINS]), &view, 0.0);
        twice.fill(&mut paint, &cfg, Some(&loud), &view, 0.05);
        twice.fill(&mut paint, &cfg, Some(&loud), &view, 0.05);
        assert_eq!(paint.levels[1000], stepped, "the second draw of one frame stepped it again");
    }

    /// The analyzer falling silent is a target of SILENCE rather than an early
    /// return, so the reading leaves the way it arrived instead of dropping to
    /// the ramp's floor in the one frame `display` stops answering.
    #[test]
    fn no_audio_leaves_on_the_release_rather_than_at_once() {
        let view = timed(0.01, 1.0);
        let cfg = crate::SpectrumConfig::default();
        let mut ring = RingLevels::default();
        let mut paint = SpectralPaint::new(&view, Default::default());
        ring.fill(&mut paint, &cfg, Some(&[1.0; SPECTRUM_BINS]), &view, 0.0);
        let lit = paint.levels[1000];
        assert!(lit > 200, "the fixture did not light the ring to begin with");
        ring.fill(&mut paint, &cfg, None, &view, 0.05);
        let after = paint.levels[1000];
        assert!(after < lit, "a silent analyzer has to start the reading down");
        assert!(after > 200, "and on a 1 s release it must not have arrived, but it reads {after}");
    }

    /// Zero is the off position and lands every reading outright, which is what
    /// makes the bars' floors the picture with no envelope in them.
    #[test]
    fn a_time_of_zero_lands_the_reading_at_once() {
        let view = timed(0.0, 0.0);
        let up = carried(&view, 0.0, 1.0, 0.008, 1);
        let down = carried(&view, 1.0, 0.0, 0.008, 1);
        assert!(up[1] > 200, "a zero attack did not land in one frame");
        assert_eq!(down[1], 0, "a zero release did not land in one frame");
    }

    /// A TIME is the same filter however it is stepped: a tenth of a second
    /// reached in one step of 100 ms and in ten of 10 ms lands in the same
    /// place. This is the whole reason the bars hold times and the coefficient
    /// is derived, and it is what keeps a 60 fps render and a 144 Hz pane
    /// drawing one picture.
    #[test]
    fn a_time_is_the_same_filter_at_any_step_size() {
        let view = timed(0.1, 0.1);
        let coarse = *carried(&view, 0.0, 1.0, 0.1, 1).last().unwrap();
        let fine = *carried(&view, 0.0, 1.0, 0.01, 10).last().unwrap();
        assert!(
            coarse.abs_diff(fine) <= 1,
            "one 100 ms step reads {coarse} where ten 10 ms steps read {fine}",
        );
    }
}
