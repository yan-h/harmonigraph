//! The spectrogram's stored byte: a slab's mean linear power, in dB,
//! quantized to half a dB step. The history that folds columns into slabs is
//! the UI's slab pyramid; this is the encoding it and the shader share.

/// One stored bucket: power in dB, quantized to a byte. See [`quantize`].
pub type BucketDb = u8;

/// The dB a stored `0` stands for, and the step between stored values.
///
/// The pair covers -120 dB to +7.5 dB. The floor is exactly where the display's
/// own mapping bottoms out (`loudness` clamps power at 1e-12), and below the
/// -100 dB where the range bars stop, so the quietest cell the UI can ask to
/// see is above the quietest value there is — the encoding adds no floor of its
/// own, and nothing fades out early against one. The ceiling sits above a full-scale
/// sine's 0 dB, which is already saturated white at any range the bars allow,
/// so nothing visible clips against it either.
pub const DB_FLOOR: f32 = -120.0;
pub const DB_STEP: f32 = 0.5;

/// Power at or below [`DB_FLOOR`] — the many empty buckets of a typical
/// spectrum, which [`quantize_exact`] answers without reaching for a `log10`.
///
/// Also what silence MEANS to every picture of the spectrum: the display's
/// mapping clamps here too, so two buckets at or below it draw the same at any
/// setting, and a spectrum with nothing above it is one nothing on screen can
/// tell from another.
pub const POWER_FLOOR: f32 = 1e-12;

/// Store a bucket's absolute power (as `harmonigraph-analysis::SpectrumAnalyzer`
/// reports it) on the dB grid.
///
/// Answers [`quantize_exact`] bit for bit, through a table instead of a
/// `log10`. The heatmap quantizes every bin of a slab each time it finishes
/// one, at every level a column folds into, and the `log10` was most of the
/// cost of a full-window fold before it,
/// paid even for floor-level bins: vectorized, the floor test became a select
/// after a `log10` on every lane.
#[inline]
pub fn quantize(power: f32) -> BucketDb {
    let bits = power.to_bits();
    match quantize_cells().get((bits >> CELL_SHIFT) as usize) {
        Some(cell) => cell.below + BucketDb::from(bits >= cell.step),
        // Negative, infinite or NaN: rare enough to ask the function itself.
        None => quantize_exact(power),
    }
}

/// The definition [`quantize`] is a table of. Its calibration is the stored
/// format's, so it is the one to change; the table follows.
pub fn quantize_exact(power: f32) -> BucketDb {
    if power <= POWER_FLOOR {
        return 0;
    }
    let db = 10.0 * power.log10();
    ((db - DB_FLOOR) / DB_STEP).round().clamp(0.0, BucketDb::MAX as f32) as BucketDb
}

/// Low bits of an `f32` a [`QuantizeCell`] spans: it keeps the exponent and the
/// top four mantissa bits, so a cell covers a power ratio of at most 17/16,
/// 0.26 dB — under one [`DB_STEP`], so at most one output step falls inside it.
const CELL_SHIFT: u32 = 19;

/// One cell of [`quantize`]'s table: every non-negative finite `f32` whose bits
/// share the cell's top bits quantizes to `below`, or to `below + 1` from bit
/// pattern `step` on. For non-negative floats bit order is value order, so a
/// monotone [`quantize_exact`] is exactly reproduced by one threshold per cell.
#[derive(Clone, Copy)]
struct QuantizeCell {
    below: BucketDb,
    /// First bit pattern quantizing to `below + 1`; `u32::MAX` when none does.
    step: u32,
}

/// [`quantize`]'s table, built on first use. Inline so the hot loop pays only
/// the initialized check, not a call.
#[inline]
fn quantize_cells() -> &'static [QuantizeCell] {
    static CELLS: std::sync::OnceLock<Box<[QuantizeCell]>> = std::sync::OnceLock::new();
    CELLS.get_or_init(build_quantize_cells)
}

/// Bisect [`quantize_exact`] itself in each cell, so the table holds whatever
/// the platform's `log10` rounds to rather than a re-derivation of it. Covers
/// `+0.0` up to the largest finite `f32`; the infinities, NaNs and negatives
/// index past its end. About 20 thousand `log10`s, once.
fn build_quantize_cells() -> Box<[QuantizeCell]> {
    let q = |bits: u32| quantize_exact(f32::from_bits(bits));
    (0..f32::INFINITY.to_bits() >> CELL_SHIFT)
        .map(|i| {
            let (lo, hi) = (i << CELL_SHIFT, ((i + 1) << CELL_SHIFT) - 1);
            let below = q(lo);
            let above = q(hi);
            if above == below {
                return QuantizeCell { below, step: u32::MAX };
            }
            assert_eq!(above, below + 1, "a quantize cell spans more than one step");
            // Smallest bit pattern in (lo, hi] quantizing to `above`.
            let (mut ok, mut ko) = (hi, lo);
            while ok - ko > 1 {
                let mid = ko + (ok - ko) / 2;
                if q(mid) == above {
                    ok = mid;
                } else {
                    ko = mid;
                }
            }
            QuantizeCell { below, step: ok }
        })
        .collect()
}

/// The dB a stored value stands for — the inverse of [`quantize`], to within
/// half a step.
pub fn db_of(bucket: BucketDb) -> f32 {
    DB_FLOOR + bucket as f32 * DB_STEP
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Final display quantization must preserve level ordering.
    #[test]
    fn quantizing_preserves_order() {
        let powers = [0.0, 1e-11, 1e-9, 1e-6, 1e-3, 0.01, 0.1, 0.5, 1.0, 2.0, 10.0];
        for pair in powers.windows(2) {
            assert!(
                quantize(pair[0]) <= quantize(pair[1]),
                "{} -> {} but {} -> {}",
                pair[0],
                quantize(pair[0]),
                pair[1],
                quantize(pair[1]),
            );
        }
        // And it separates levels that matter: a dB apart is at least a step.
        assert!(quantize(0.1) < quantize(0.126), "1 dB apart must not collide");
    }

    /// Round-tripping a bucket must land within half a step of where it
    /// started, across the whole range the display can read.
    #[test]
    fn quantizing_round_trips_to_half_a_step() {
        // The whole span the display's mapping reads: it bottoms out at -120 dB,
        // below the range bars' -100, and 0 dB (a full-scale sine) is the top
        // of every range bar.
        for db in [-120.0f32, -110.0, -100.0, -90.0, -60.0, -30.0, -12.0, -0.5, 0.0, 6.0] {
            let power = 10.0f32.powf(db / 10.0);
            let back = db_of(quantize(power));
            assert!((back - db).abs() <= DB_STEP * 0.5 + 1e-3, "{db} dB came back as {back} dB");
        }
        // Out of range in both directions saturates rather than wrapping. The
        // top is well above a full-scale sine, and far above any ceiling the
        // bars offer, so what saturates there was already white.
        assert_eq!(quantize(0.0), 0);
        assert_eq!(quantize(1e-30), 0);
        assert_eq!(quantize(1e9), BucketDb::MAX);
        assert!(db_of(BucketDb::MAX) >= 6.0, "no headroom left above a full-scale sine");
    }

    /// The table must answer the function it replaces at every step it
    /// encodes: each cell's two ends, and both sides of its threshold. With
    /// `quantize_exact` monotone that is every input; the exhaustive test below
    /// is what checks the monotonicity on this platform's `log10`.
    #[test]
    fn the_quantize_table_steps_where_the_function_does() {
        let cells = quantize_cells();
        let mut steps = 0;
        for (i, cell) in cells.iter().enumerate() {
            let lo = (i as u32) << CELL_SHIFT;
            let hi = lo | ((1 << CELL_SHIFT) - 1);
            let at = |bits: u32| {
                let p = f32::from_bits(bits);
                assert_eq!(quantize(p), quantize_exact(p), "{p:e} ({bits:#x})");
            };
            at(lo);
            at(hi);
            if cell.step != u32::MAX {
                steps += 1;
                at(cell.step - 1);
                at(cell.step);
            }
        }
        // One step per code: 0 to 255, the floor's own jump to 0 included.
        assert_eq!(steps, BucketDb::MAX as usize, "every output step lies inside a cell");
        // Past the table's end: the function itself answers.
        for p in [-0.0, -1.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN, -f32::NAN] {
            assert_eq!(quantize(p), quantize_exact(p), "{p}");
        }
    }

    /// Every `f32` bit pattern, in parallel: a few seconds in release, which
    /// is why it is not in the default run. Run it after touching `quantize`
    /// or on a new platform: `cargo test --release -p harmonigraph-core
    /// quantize_matches_its_definition_everywhere -- --ignored`.
    #[test]
    #[ignore]
    fn quantize_matches_its_definition_everywhere() {
        let threads = std::thread::available_parallelism().map_or(8, |n| n.get()) as u64;
        let all = 1u64 << 32;
        std::thread::scope(|s| {
            for t in 0..threads {
                s.spawn(move || {
                    for bits in (all * t / threads)..(all * (t + 1) / threads) {
                        let p = f32::from_bits(bits as u32);
                        assert_eq!(quantize(p), quantize_exact(p), "{p:e} ({bits:#x})");
                    }
                });
            }
        });
    }
}
