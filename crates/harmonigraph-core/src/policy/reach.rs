//! Non-realtime analytic reachability over an explicitly bounded input register.
//! Equal-curvature pitch parabolas differ linearly on each octave interval.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub config: MusicalConfig,
    pub reference: i64,
    pub context: Vec<ContextPitch>,
}
#[derive(Clone, Copy)]
struct Piece {
    node: LatticePos,
    center: f64,
    low: f64,
    high: f64,
    cost: f64,
}
/// `cancelled` lets a display worker abandon obsolete context. Never call this
/// allocating calculation from the audio thread.
pub fn reachable(
    snapshot: &Snapshot,
    low: f64,
    high: f64,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<LatticePos>, InputError> {
    if !low.is_finite() || !high.is_finite() || high <= low || high - low > 12_000.0 {
        return Err(InputError::InvalidPitch);
    }
    let c = snapshot.config;
    let reference = snapshot.reference as f64 / 1_000_000.0;
    let mut scratch = PolicyScratch::default();
    prepare(c, &snapshot.context, &mut scratch)?;
    let mut pieces = Vec::new();
    for &node in &scratch.candidates {
        let base = c.cents(node);
        let first = ((low + reference - 600.0 - base) / 1200.0).ceil() as i64;
        let last = ((high + reference + 600.0 - base) / 1200.0).floor() as i64;
        for octave in first..=last {
            let output = base + 1200.0 * octave as f64;
            let center = output - reference;
            let lo = low.max(center - 600.0);
            let hi = high.min(center + 600.0);
            if hi > lo {
                pieces.push(Piece {
                    node,
                    center,
                    low: lo,
                    high: hi,
                    cost: harmonic_cost(c, node, output, &scratch.context),
                });
            }
        }
        if cancelled() {
            return Ok(Vec::new());
        }
    }
    let scale2 = f64::from(c.policy.pitch_scale).powi(2);
    let mut result = Vec::new();
    let mut boundaries = vec![low, high];
    for a in &pieces {
        if cancelled() {
            return Ok(Vec::new());
        }
        boundaries.extend([a.low, a.high]);
        let mut ranges = vec![(a.low, a.high)];
        for b in &pieces {
            if std::ptr::eq(a, b) || b.high <= a.low || b.low >= a.high {
                continue;
            }
            let slope = 2.0 * (b.center - a.center) / scale2;
            let intercept =
                (a.center - b.center) * (a.center + b.center) / scale2 + a.cost - b.cost;
            let (mut lo, mut hi) = (b.low, b.high);
            if slope.abs() < 1e-12 {
                if intercept < -1e-10 || (intercept.abs() <= 1e-10 && key(a.node) <= key(b.node)) {
                    continue;
                }
            } else if slope > 0.0 {
                lo = lo.max(-intercept / slope);
            } else {
                hi = hi.min(-intercept / slope);
            }
            if hi <= lo {
                continue;
            }
            let mut next = Vec::new();
            for (rlo, rhi) in ranges {
                if hi <= rlo || lo >= rhi {
                    next.push((rlo, rhi));
                } else {
                    if lo > rlo {
                        next.push((rlo, rhi.min(lo)));
                    }
                    if hi < rhi {
                        next.push((rlo.max(hi), rhi));
                    }
                }
            }
            ranges = next;
            if ranges.is_empty() {
                break;
            }
        }
        if !ranges.is_empty() {
            result.push(a.node);
            for (lo, hi) in ranges {
                boundaries.extend([lo, hi]);
            }
        }
    }
    boundaries.sort_by(f64::total_cmp);
    boundaries.dedup();
    for input in boundaries {
        if cancelled() {
            return Ok(Vec::new());
        }
        if let Some(node) = select_prepared(
            c,
            snapshot.reference,
            OrderedOnset { pitch: (input * 1_000_000.0).round() as i64 },
            &scratch,
        )?
        .assignment
        .node()
        {
            result.push(node);
        }
    }
    result.sort_by_key(|n| key(*n));
    result.dedup();
    Ok(result)
}
