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
/// policy: with the band index below a slot is 1280 bytes, 5.2 MB for a
/// whole-song ring beside the grid's own 15.7 MB. Musically it is far past a
/// chord's worth of partials — a five-note chord with eight audible partials
/// each is 40 — and what it cuts off under a dense mix is the quietest of
/// them, which is what the prominence ordering is for.
pub(super) const MAX_PEAKS: usize = 48;

/// Buckets one band of the index below covers: a semitone at the analyzer's
/// own 32 buckets to one.
pub(super) const PEAK_BAND_BUCKETS: usize = 32;

/// Bands the index carries, covering buckets `0 .. PEAK_BANDS *
/// PEAK_BAND_BUCKETS` — 4064, past the 3828 the analyzer's axis holds.
///
/// A slab longer than that is still drawn correctly: its top buckets fall in
/// the last band and are reached by scanning forward from it, which costs more
/// loads and answers the same picture.
pub(super) const PEAK_BANDS: usize = 127;

/// `vec4`s the header occupies: `PEAK_BANDS + 1` entries packed four to a
/// `vec4`, which is 128 entries in exactly 32 of them.
///
/// **The layout.** Entry `k` lives at `vec4` `k >> 2`, component `k & 3`, and
/// holds the INDEX of the first stored peak with `x >= k * PEAK_BAND_BUCKETS`
/// — so it is non-decreasing in `k`, and entry [`PEAK_BANDS`] is the peak
/// COUNT (there being no peak past the last band). Peaks follow from `vec4`
/// [`PEAK_HEADER`], sorted by `x` ascending.
///
/// Exact in an `f32` because every value is an integer under 49.
///
/// What it buys is the whole reason the time gather can be wide: a fragment
/// covers one pitch, so all but a peak or two of a slab's 48 are nowhere near
/// it. Without the index every gathered slab costs 48 loads and 48 Gaussians
/// per fragment, and a five-slab gather is 240 of them; with it a slab costs
/// the peaks inside `[x - 3 sigma, x + 3 sigma]` plus at most one band of
/// lead-in, which over a 3828-bucket slab holding 48 peaks is about two.
pub(super) const PEAK_HEADER: usize = (PEAK_BANDS + 1).div_ceil(4);

/// `vec4`s one slot occupies: the header, then [`MAX_PEAKS`] entries. The
/// shader indexes `slot * PEAK_STRIDE`.
pub(super) const PEAK_STRIDE: usize = PEAK_HEADER + MAX_PEAKS;

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
/// Returned SORTED BY `x` ascending, which is what lets the band index in
/// [`write_slot`] hand a fragment a starting point and lets the scan stop at
/// the first peak past its reach.
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
        // The scan emitted them in bucket order and the select has just
        // scrambled the survivors. Put them back: the band index and the
        // shader's early break both read this order, not the prominence one.
        found.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
    }
    found
}

/// Write `slab`'s peaks into `out`, one slot's [`PEAK_STRIDE`] `vec4`s: the
/// band index described at [`PEAK_HEADER`], then the entries, then zeros.
///
/// The tail is cleared rather than left: a slot reused by a later key must not
/// answer with the peaks of the slab it held a lap ago.
pub(super) fn write_slot(slab: &[u8], out: &mut [[f32; 4]]) {
    debug_assert_eq!(out.len(), PEAK_STRIDE, "a slot is exactly one peak record");
    let found = peaks_of(slab);
    out.fill([0.0; 4]);
    out[PEAK_HEADER..PEAK_HEADER + found.len()].copy_from_slice(&found);
    // One pass up the sorted list: `at` only ever moves forward, so the whole
    // index costs the peaks themselves plus the bands.
    let mut at = 0usize;
    for k in 0..PEAK_BANDS {
        let edge = (k * PEAK_BAND_BUCKETS) as f32;
        while at < found.len() && found[at][0] < edge {
            at += 1;
        }
        out[k >> 2][k & 3] = at as f32;
    }
    out[PEAK_BANDS >> 2][PEAK_BANDS & 3] = found.len() as f32;
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
        // The cap is also where the ascending order is easiest to lose: the
        // select that answers the question above leaves its survivors
        // scrambled, and the band index and the shader's early break both
        // read the order rather than re-deriving it.
        assert!(
            found.windows(2).all(|pair| pair[0][0] < pair[1][0]),
            "the kept peaks came back out of bucket order",
        );
    }

    /// A slot's record is the band index and then the peaks, with the tail
    /// cleared — a slot the ring reuses must not answer with a lap-old list.
    #[test]
    fn a_written_slot_clears_the_peaks_it_used_to_hold() {
        let mut slot = vec![[1.0f32; 4]; PEAK_STRIDE];
        let mut slab = vec![0u8; 200];
        slab[100] = 200;
        write_slot(&slab, &mut slot);
        assert_eq!(band(&slot, PEAK_BANDS), 1, "one peak, and the header says so");
        assert!((slot[PEAK_HEADER][0] - 100.5).abs() < 0.01);
        assert!(
            slot[PEAK_HEADER + 1..].iter().all(|entry| *entry == [0.0; 4]),
            "the tail still holds what the slot did before",
        );
    }

    /// Header entry `k`, read the way the shader reads it.
    fn band(slot: &[[f32; 4]], k: usize) -> usize {
        slot[k >> 2][k & 3] as usize
    }

    /// The band index lands a scan at the right peak.
    ///
    /// The claim is the one the shader depends on: entry `k` is the first
    /// stored peak at or past bucket `32k`, so a fragment that starts there
    /// cannot step over a peak that covers it. Checked against a linear search
    /// over EVERY band rather than at a couple of hand-picked ones — the ways
    /// this goes wrong (an off-by-one at a band edge, a peak landing exactly
    /// on one, an empty band inheriting the wrong index) are all at edges, and
    /// naming three of them by hand is how the fourth survives.
    ///
    /// The fixture reaches past one band: peaks are spread over 1500 buckets,
    /// which is 47 of them, with runs of empty bands between.
    #[test]
    fn the_band_index_lands_a_scan_at_the_first_peak_of_its_band() {
        let mut slab = vec![0u8; 1500];
        // Deliberately uneven spacing, including two peaks inside one band and
        // one sitting on a band edge (bucket 640 = 32 * 20).
        for at in [7, 200, 210, 640, 641 + 16, 900, 1400] {
            slab[at] = 180;
        }
        let found = peaks_of(&slab);
        assert!(found.len() >= 6, "the fixture lost its peaks: {}", found.len());
        let mut slot = vec![[0.0f32; 4]; PEAK_STRIDE];
        write_slot(&slab, &mut slot);
        assert_eq!(band(&slot, PEAK_BANDS), found.len(), "the sentinel is not the count");
        for k in 0..PEAK_BANDS {
            let edge = (k * PEAK_BAND_BUCKETS) as f32;
            let want = found.iter().take_while(|peak| peak[0] < edge).count();
            assert_eq!(band(&slot, k), want, "band {k} starts at the wrong peak");
        }
    }

    /// What picking a whole run's peaks costs, which is what a FULL UPLOAD
    /// pays synchronously inside `prepare`.
    ///
    /// The steady-state frame is not this: it patches the slab or two that
    /// moved, and `what_partials_detail_costs_a_frame` in
    /// `harmonigraph-offline` is what that costs. This lands instead on a
    /// refold, a capacity change, the first frame of a surface, and the frame
    /// the reader switches the detail on — every one of which already rewrites
    /// the whole grid beside it.
    ///
    /// Both caps are measured because they are two different stalls: the live
    /// ring's 1024 slabs is a frame the reader is dragging a Span through, and
    /// the whole-song 4096 is once per offline render.
    ///
    /// `#[ignore]`, and it asserts nothing: the numbers belong to the machine
    /// that ran them.
    #[test]
    #[ignore]
    fn what_picking_a_full_run_costs() {
        // A bed with a partial every 61 buckets — dense enough that the cap
        // binds on every slab, so this is the worst case rather than a quiet
        // passage.
        let mut slab = vec![0u8; 3828];
        let mut seed = 0x2545_f491u32;
        for (j, byte) in slab.iter_mut().enumerate() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *byte = (40 + ((seed >> 24) % 40) + u32::from(j % 61 == 0) * 90).min(255) as u8;
        }
        let mut record = vec![[0f32; 4]; PEAK_STRIDE];
        // A throwaway pass in front, for the weight table's one-time build and
        // the first touch of the slab. Without it the FIRST row pays for both
        // and reads a third high, which is a difference between the two rows
        // that has nothing to do with their size.
        write_slot(&slab, &mut record);
        eprintln!("\n== picking a full run, {} buckets a slab ==", slab.len());
        for slabs in [1024usize, 4096] {
            let start = std::time::Instant::now();
            for _ in 0..slabs {
                write_slot(&slab, &mut record);
            }
            let elapsed = start.elapsed().as_secs_f64();
            eprintln!(
                "{slabs:>5} slabs: {:>7.1} ms ({:.1} us a slab)",
                elapsed * 1e3,
                elapsed * 1e6 / slabs as f64,
            );
        }
        eprintln!();
    }
}
