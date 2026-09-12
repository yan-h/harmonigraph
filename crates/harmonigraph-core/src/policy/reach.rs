//! Non-realtime analytic reachability over an explicitly bounded input register.
//! Shifted exponential pitch costs cross at most once on each overlap.
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
    // A key that loses to passthrough still excludes other key classes. Build
    // the occupied windows from every candidate, before clipping by benefit.
    let mut occupied = Vec::new();
    for &node in &scratch.candidates {
        occupied.extend(key_windows(c, node, low, high));
    }
    occupied.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut gaps = Vec::new();
    let mut cursor = low;
    for (lo, hi) in occupied {
        if lo > cursor {
            gaps.push((cursor, lo));
        }
        cursor = cursor.max(hi);
    }
    if cursor < high {
        gaps.push((cursor, high));
    }
    let flexibility = c.policy.pitch_flexibility;
    let max_radius = f64::from(flexibility);
    let mut pieces = Vec::new();
    for &node in &scratch.candidates {
        let base = c.cents(node);
        let first = ((low + reference - max_radius - base) / 1200.0).ceil() as i64;
        let last = ((high + reference + max_radius - base) / 1200.0).floor() as i64;
        let mut eligible = gaps.clone();
        eligible.extend(key_windows(c, node, low, high));
        for octave in first..=last {
            let output = ((base + 1200.0 * octave as f64) * 1_000_000.0).round() / 1_000_000.0;
            let center = output - reference;
            let benefit = harmonic_benefit(c, node, output, &scratch.context);
            let radius = benefit_radius(flexibility, benefit);
            for &(lo, hi) in &eligible {
                let lo = lo.max(center - radius);
                let hi = hi.min(center + radius);
                if hi > lo {
                    pieces.push(Piece { node, center, low: lo, high: hi, cost: -benefit });
                }
            }
        }
        if cancelled() {
            return Ok(Vec::new());
        }
    }
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
            let (mut lo, mut hi) = (b.low.max(a.low), b.high.min(a.high));
            if a.center == b.center {
                if a.cost < b.cost || (a.cost == b.cost && key(a.node) <= key(b.node)) {
                    continue;
                }
            } else {
                let difference = |x| {
                    pitch_cost(flexibility, x - a.center) - pitch_cost(flexibility, x - b.center)
                        + a.cost
                        - b.cost
                };
                let (left, right) = (difference(lo), difference(hi));
                if left <= 0.0 && right <= 0.0 {
                    continue;
                }
                if left < 0.0 || right < 0.0 {
                    // P is strictly convex, so two translated copies have a
                    // strictly monotone difference. Bisect their sole crossing
                    // to finer than the input's one-microcent resolution.
                    let increasing = a.center < b.center;
                    let (mut lower, mut upper) = (lo, hi);
                    for _ in 0..48 {
                        let middle = (lower + upper) * 0.5;
                        if (difference(middle) < 0.0) == increasing {
                            lower = middle;
                        } else {
                            upper = middle;
                        }
                    }
                    let crossing = (lower + upper) * 0.5;
                    if increasing {
                        lo = crossing;
                    } else {
                        hi = crossing;
                    }
                }
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
            for (lo, hi) in ranges {
                boundaries.extend([lo, (lo + hi) * 0.5, hi]);
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

/// Input windows in which this node belongs to the preferred keyboard pool.
fn key_windows(config: MusicalConfig, node: LatticePos, low: f64, high: f64) -> Vec<(f64, f64)> {
    let center = (i64::from(config.c_offset) + keyboard_class(config.policy.keyboard, node)) as f64
        / 1_000_000.0;
    let tolerance = KEYBOARD_TOLERANCE as f64 / 1_000_000.0;
    let first = ((low - tolerance - center) / 1200.0).ceil() as i64;
    let last = ((high + tolerance - center) / 1200.0).floor() as i64;
    (first..=last)
        .map(|octave| {
            let center = center + 1200.0 * octave as f64;
            (low.max(center - tolerance), high.min(center + tolerance))
        })
        .collect()
}
