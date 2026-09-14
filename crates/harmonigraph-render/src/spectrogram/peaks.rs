//! Each slab's local maxima, picked once where the slab is uploaded.
//!
//! **A pure function of the slab's bytes.** No setting reaches it — not the
//! stroke width, not the prominence bar, not the pitch range — so the peak
//! list is keyed on exactly what the grid itself is keyed on and there is no
//! second cache key to get wrong. A bar the reader drags moves uniforms, and
//! the fragment shader decides which of these peaks to draw and how wide.
//!
//! The two numbers that are NOT config, and so live here: how far a bucket has
//! to beat its neighbours to be a maximum at all ([`REACH`]), and how wide the
//! neighbourhood a peak's prominence is measured against is ([`MEAN_REACH`]).
//! Both are properties of the analyzer's own grid — 32 buckets to a semitone —
//! rather than of the picture.

/// Buckets either side a bucket must beat to count as a peak.
///
/// Six is under a quarter of a semitone, so two partials a semitone apart are
/// two peaks; it is also wide enough that the estimator's own ripple around a
/// line does not read as a row of them.
const REACH: usize = 6;

/// Buckets either side of a peak the local mean is taken over.
///
/// A whole semitone: wide enough that a partial does not raise its own
/// baseline out from under itself, narrow enough that the mean is the haze
/// AROUND this partial rather than the column's average level. That is what
/// makes the prominence bar read the same over a quiet passage as a loud one.
const MEAN_REACH: usize = 32;

/// Peaks kept per slab.
///
/// The buffer is a fixed record per slot, so this is a size as well as a
/// policy: 48 of them is 784 bytes a slot, 3.2 MB for a whole-song ring beside
/// the grid's own 15.7 MB. Musically it is far past a chord's worth of
/// partials — a five-note chord with eight audible partials each is 40 — and
/// what it cuts off under a dense mix is the quietest of them, which is what
/// the prominence ordering is for.
pub(super) const MAX_PEAKS: usize = 48;

/// `vec4`s one slot occupies in the peaks buffer: a header holding the count,
/// then [`MAX_PEAKS`] entries. The shader indexes `slot * PEAK_STRIDE`.
pub(super) const PEAK_STRIDE: usize = MAX_PEAKS + 1;

/// dB one stored step of the grid carries.
///
/// The grid is bytes of 0.5 dB (`harmonigraph_core::spectrogram::DB_STEP`),
/// and this is the one place in this crate that has to know it: the centroid
/// below weighs its taps by POWER, which is a dB ratio. Everything else reads
/// a byte through the affine the caller hands over. `read_of` on the UI side
/// converts the prominence bar through this same constant, and asserts the two
/// definitions agree.
pub const GRID_DB_PER_STEP: f32 = 0.5;

/// The centroid's weight for a tap `d` steps BELOW the peak it belongs to:
/// `10^(-d * GRID_DB_PER_STEP / 10)`, the power ratio the two dB readings
/// stand in.
///
/// A table because the argument is an integer count of steps and there are
/// only 256 of them, while the call is per tap per peak — thirteen taps for
/// every local maximum in every slab of the run. Over a 2600-slab whole-song
/// fold that is 87 ms with `powf` against 79 with the table, and 67 once the
/// cap's sort became a select below.
///
/// Only the below-the-peak direction exists: every tap in the window is at or
/// under the peak by construction, which is what made it a peak.
static CENTROID_WEIGHT: std::sync::LazyLock<[f32; 256]> = std::sync::LazyLock::new(|| {
    std::array::from_fn(|d| 10f32.powf(-(d as f32) * GRID_DB_PER_STEP / 10.0))
});

/// The peaks `slab` holds: at most [`MAX_PEAKS`] of them, the ones standing
/// furthest above their own local mean.
///
/// Each is `(x, byte, mean, 0)`, where `x` is the peak's position on the
/// CONTINUOUS bucket axis the shader's `bucket_x` returns — bucket `b` spans
/// `[b, b + 1)`, so bucket `b`'s own centre is `b + 0.5` and a peak between
/// two buckets lands between them. `byte` and `mean` are in stored steps, so
/// `byte - mean` is the prominence the shader compares against its threshold
/// and `byte` goes through the same level affine a bucket does.
///
/// Returned in no particular order: the shader takes a max over the list.
pub(super) fn peaks_of(slab: &[u8]) -> Vec<[f32; 4]> {
    let n = slab.len();
    if n == 0 {
        return Vec::new();
    }
    let mut found: Vec<[f32; 4]> = Vec::new();
    // The local mean by a running sum over the clamped window, so the whole
    // scan is linear in the slab rather than `n * MEAN_REACH`. `lo` and `hi`
    // are the window's inclusive ends and only ever move forward.
    let mut lo = 0usize;
    let mut hi = MEAN_REACH.min(n - 1);
    let mut sum: u32 = slab[lo..=hi].iter().map(|&b| u32::from(b)).sum();
    for i in 0..n {
        let want_hi = (i + MEAN_REACH).min(n - 1);
        while hi < want_hi {
            hi += 1;
            sum += u32::from(slab[hi]);
        }
        let want_lo = i.saturating_sub(MEAN_REACH);
        while lo < want_lo {
            sum -= u32::from(slab[lo]);
            lo += 1;
        }
        let byte = slab[i];
        // Silence is not a peak however flat its surroundings are, and the
        // test is what keeps an empty slab's list empty rather than full of
        // the leftmost zero of every plateau.
        //
        // Nor is a bucket with no room for the comparison. A window clipped by
        // the slab's edge would let the very first bucket of a flat floor pass
        // as the left end of one enormous plateau, which is a peak nothing
        // measured — so the outermost [`REACH`] buckets at each end are not
        // candidates. They are the analyzer's own extremes, under 20 Hz and
        // over 20 kHz, where a partial is not a reading anyone takes.
        if byte == 0 || i < REACH || i + REACH >= n {
            continue;
        }
        let (left, right) = (i - REACH, i + REACH);
        // A flat top counts ONCE, at its left end: a tie to the left
        // disqualifies, a tie to the right does not. Without the asymmetry a
        // plateau spends one of the 48 slots per bucket of itself.
        if slab[left..i].iter().any(|&b| b >= byte) || slab[i + 1..=right].iter().any(|&b| b > byte)
        {
            continue;
        }
        // Power-weighted centroid over the peak's own neighbourhood: steadier
        // column to column than a three-point parabola on noisy dB, which is
        // what the prototype measured and what stops a partial's line wobbling
        // by a bucket a frame.
        let (mut num, mut den) = (0.0f32, 0.0f32);
        for j in left..=right {
            let w = CENTROID_WEIGHT[usize::from(byte - slab[j])];
            num += w * (j as f32 + 0.5);
            den += w;
        }
        let mean = sum as f32 / (hi - lo + 1) as f32;
        found.push([num / den, f32::from(byte), mean, 0.0]);
    }
    if found.len() > MAX_PEAKS {
        // A SELECT and not a sort: the shader takes a max over the list, so
        // which 48 survive is the whole question and their order is not one.
        // Two peaks are always at least a bucket apart, so ordering the tie by
        // position makes the comparison a strict total order and the surviving
        // SET the same on every run — which is what a golden frame needs.
        found.select_nth_unstable_by(MAX_PEAKS, |a, b| {
            (b[1] - b[2]).total_cmp(&(a[1] - a[2])).then(a[0].total_cmp(&b[0]))
        });
        found.truncate(MAX_PEAKS);
    }
    found
}

/// Write `slab`'s peaks into `out`, one slot's [`PEAK_STRIDE`] `vec4`s: the
/// count in the header's first lane, then the entries, then zeros.
///
/// The tail is cleared rather than left: a slot reused by a later key must not
/// answer with the peaks of the slab it held a lap ago.
pub(super) fn write_slot(slab: &[u8], out: &mut [[f32; 4]]) {
    debug_assert_eq!(out.len(), PEAK_STRIDE, "a slot is exactly one peak record");
    let found = peaks_of(slab);
    out[0] = [found.len() as f32, 0.0, 0.0, 0.0];
    out[1..1 + found.len()].copy_from_slice(&found);
    out[1 + found.len()..].fill([0.0; 4]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic noise bed, so a fixture draws the same buckets on every
    /// machine and the peak COUNT it produces is a number the test can reason
    /// about. The same LCG the render crate's other fixtures use.
    fn bed(slab: &mut [u8], floor: u8, spread: u32) {
        let mut seed = 0x2545_f491u32;
        for byte in slab.iter_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *byte = floor + ((seed >> 24) % spread) as u8;
        }
    }

    /// Add a Gaussian in dB — a partial as the analyzer draws one, a line
    /// spread over the estimator's main lobe — centred on the bucket axis at
    /// `centre` and `amplitude` stored steps tall.
    fn partial(slab: &mut [u8], centre: f32, amplitude: f32, sigma: f32) {
        for (j, byte) in slab.iter_mut().enumerate() {
            let d = j as f32 + 0.5 - centre;
            let lift = amplitude * (-(d * d) / (2.0 * sigma * sigma)).exp();
            *byte = (f32::from(*byte) + lift).round().clamp(0.0, 255.0) as u8;
        }
    }

    fn prominence(peak: [f32; 4]) -> f32 {
        peak[1] - peak[2]
    }

    /// Two partials over a noise floor: both are found, the louder stands
    /// further above its surroundings, and nothing the bed threw up stands as
    /// far as either.
    ///
    /// The bed is what makes it a measurement rather than an assertion about
    /// two isolated spikes: it really does produce local maxima (the count is
    /// checked, so a bed that quietly stopped producing them would fail rather
    /// than pass for the wrong reason), and the claim is that the PICKER's
    /// ordering separates them from the partials.
    #[test]
    fn two_partials_over_a_noise_bed_outrank_everything_the_bed_threw_up() {
        let mut slab = vec![0u8; 400];
        bed(&mut slab, 40, 9);
        partial(&mut slab, 100.5, 60.0, 2.5);
        partial(&mut slab, 300.5, 20.0, 2.5);
        let found = peaks_of(&slab);
        let near = |x: f32| {
            *found
                .iter()
                .filter(|p| (p[0] - x).abs() < 1.0)
                .max_by(|a, b| prominence(**a).total_cmp(&prominence(**b)))
                .unwrap_or_else(|| panic!("no peak within a bucket of {x}"))
        };
        let (loud, quiet) = (near(100.5), near(300.5));
        assert!(
            prominence(loud) > prominence(quiet),
            "the 30 dB partial ({:.1}) did not outrank the 10 dB one ({:.1})",
            prominence(loud),
            prominence(quiet),
        );
        let bed_peaks: Vec<_> = found
            .iter()
            .copied()
            .filter(|p| (p[0] - 100.5).abs() >= 1.0 && (p[0] - 300.5).abs() >= 1.0)
            .collect();
        let worst = bed_peaks.iter().copied().map(prominence).fold(f32::MIN, f32::max);
        assert!(
            bed_peaks.len() >= 8,
            "the bed produced {} local maxima, too few for the ordering to be measured against",
            bed_peaks.len(),
        );
        assert!(
            worst < prominence(quiet),
            "a bed maximum ({worst:.1}) stood as far above its surroundings as the quiet partial \
             ({:.1})",
            prominence(quiet),
        );
    }

    /// The centroid lands on the partial's own centre, not on the bucket that
    /// happens to hold its top.
    ///
    /// The centre sits 0.4 of a bucket off a bucket centre, which is the whole
    /// of what the weighting buys: a picker that answered the peak bucket's
    /// centre would be out by exactly that, four times the tolerance here.
    #[test]
    fn the_centroid_lands_on_a_synthetic_partials_centre() {
        const CENTRE: f32 = 200.9;
        let mut slab = vec![0u8; 400];
        partial(&mut slab, CENTRE, 80.0, 1.5);
        let found = peaks_of(&slab);
        assert_eq!(found.len(), 1, "a lone partial is one peak");
        assert!(
            (found[0][0] - CENTRE).abs() < 0.1,
            "the centroid read {:.3}, not {CENTRE}",
            found[0][0],
        );
    }

    /// A flat top is one peak, at its left end — and so costs one of the 48
    /// slots rather than one per bucket of itself.
    #[test]
    fn a_flat_top_yields_one_peak() {
        let mut slab = vec![20u8; 200];
        slab[100..105].fill(200);
        let found = peaks_of(&slab);
        assert_eq!(found.len(), 1, "a five-bucket plateau was picked {} times", found.len());
        assert!(
            (100.0..105.0).contains(&found[0][0]),
            "the plateau's peak landed at {:.2}, outside it",
            found[0][0],
        );
    }

    /// Past the cap it is the most prominent that are kept.
    ///
    /// Sixty peaks, a bucket wide and twenty apart so every one of them really
    /// is a local maximum, rising in height along the slab — which makes the
    /// prominence ordering the height ordering (the local mean rises far more
    /// slowly than the peak does) and the kept set nameable.
    #[test]
    fn the_cap_keeps_the_most_prominent() {
        const PEAKS: usize = 60;
        let mut slab = vec![0u8; 20 * PEAKS + 20];
        for k in 0..PEAKS {
            slab[10 + 20 * k] = 60 + k as u8;
        }
        const { assert!(PEAKS > MAX_PEAKS, "the fixture has to overflow the cap to measure it") };
        let found = peaks_of(&slab);
        assert_eq!(found.len(), MAX_PEAKS);
        let lowest = found.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert_eq!(
            lowest,
            (60 + PEAKS - MAX_PEAKS) as f32,
            "the kept set is not the tallest {MAX_PEAKS}",
        );
    }

    /// A slot's record is the count and then the peaks, with the tail cleared
    /// — a slot the ring reuses must not answer with a lap-old list.
    #[test]
    fn a_written_slot_clears_the_peaks_it_used_to_hold() {
        let mut slot = vec![[1.0f32; 4]; PEAK_STRIDE];
        let mut slab = vec![0u8; 200];
        slab[100] = 200;
        write_slot(&slab, &mut slot);
        assert_eq!(slot[0][0], 1.0, "one peak, and the header says so");
        assert!((slot[1][0] - 100.5).abs() < 0.01);
        assert!(
            slot[2..].iter().all(|entry| *entry == [0.0; 4]),
            "the tail still holds what the slot did before",
        );
    }
}
