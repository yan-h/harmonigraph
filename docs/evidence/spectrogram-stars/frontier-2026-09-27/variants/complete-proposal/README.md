# Native complete-response prototype — restricted research eligibility

**This is not a production gate and not a bit-exact change.** No repository edit,
build or GPU validation was performed. native-complete.patch is an unapplied research.rs-only patch; regenerate with make_patch.py against the current checkout.

**Eligibility is deliberately restricted:** a depth's requested factor must be exactly 1, base halo dimensions must equal cloud.size*ppp exactly, the pane origin must align to physical pixels, and PPP must be exactly 1, 2 or 4. The shader checks these conditions in both writer and reader. Every other depth/frame retains the original residual-halo + analytic-core path. Matching target dimensions alone is insufficient: fractional pane origins can shift the sampling phase across every pixel, even at a nominal full-resolution setting.

The common integral/power-of-two image fixtures qualify. No claim is made that this covers every live window scale or layout. The experiment asks whether the upside justifies further alignment/generalization work; it does not introduce a new shipping precision/alignment contract.

## Concrete change and credible savings

For an eligible depth, its existing halo pass evaluates each of its same nine stars' complete analytic response: bounded Gaussian+fringe, original outer fade,
packed life fade, and premultiplied palette color. It omits the compact-core taper and subtraction. The original nine-add order and per-star saturation remain.

The native composite loads this complete premultiplied value directly at its matching integer pixel, then applies the unchanged per-depth normalization and far-to-near over. It skips that depth's atlas core load, coordinate/index walk,
Gaussian and core taper. Reduced depths retain both their current residual halo sampling expression and their analytic native core. Nothing pre-normalizes the stored color or clamps the per-depth summed coverage to one.

This is different from moving a nine-neighbor evaluation into the final draw:
the five depth passes and native 2+3 composition split remain. There are no new targets, bindings, passes or timestamp queries. Savings are not negligible by construction: each native depth avoids up to nine compact-core taper evaluations in its bake and one entire native-core evaluation per composite pixel. Actual gains must cover the uniform eligibility/selection branch and be measured.

## Why the analytic identity holds

The compact core is confined to its own nominal cell by reach=(1-jitter_width)/2.
No other neighboring cell's compact core contributes inside that cell. Its maximum reach is 0.5 cells, smaller than the outer halo fade's start at 0.84.
Inside core support the full response is at least the Gaussian, hence at least the tapered core. Therefore sum(full-core)+the nominal core equals sum(full),
including Jitter endpoints. Silent atlas records, lifetime fades, fringe=0,
per-star saturation, palette colors and depth order keep their existing meaning.

At the restricted aligned scales, fragment centers in the halo pass correspond to the final draw's pane points (and the split far pass's points). A direct texel load therefore selects the same analytic point. An arbitrary fractional origin would instead filter the halo between samples while keeping the old core analytic;
moving that core into the filtered image would alter the picture, hence fallback.

## What is still different numerically

Previously the target stored Q16(sum(residual)), then the native core was added in f32. Now it stores Q16(sum(complete)). Their real-valued results agree, but half rounding occurs after different arithmetic. Tiny fade values can also underflow in the complete target where the old core remained f32. A texel load replaces center sampling, removing any normalized-UV filtering-roundoff contribution.
Do not claim byte identity or a universal one-code bound before measuring.

## Cases and existing target layouts

Uniform-array examples: full-complete, full-complete-mix4, full-complete-mix5,
full-complete-v50_50_80_100_50 (P2), full-complete-v50_50_100_100_60 (P3).

The current grouped/separate hookups also accept:

- full-grouped-complete-v100_100_100_100_100
- full-grouped-complete-v50_50_80_100_50
- full-grouped-complete-v50_50_100_100_60
- full-separate-complete-v50_50_80_100_50
- full-separate-complete-v50_50_100_100_60

Grouped/separate names retain their allocation and sampling maps; eligible depths load from the matching actual array/local layer or separate texture. Reduced depths retain the previous normalized sampler helper verbatim. Every source replacement asserts the expected occurrence count. Compute/fusion/u8 combinations are excluded from this isolated experiment.

Compare each complete case to its matching layout/vector control, not a different array organization. P2 only replaces depth 3; P3 replaces depths 2 and 3. A grouped all-native control tests all five depths and gives the clearest upside bound.

## Bounded verification before interpreting performance

Render default and extreme Jitter/Fringe/Defocus/size controls, silence, near- silence, and a short lifetime/drift sequence. Compare full/complete against the independent complete nine-neighbor reference as well as the current residual control. Check both sides of the native far-split cutoff. Use PPP=2 or4 for the eligible path and PPP=1.25 as a simple fallback witness; a fractional pixel pane origin is another fallback witness where available. Report actual output error,
especially near saturated overlaps and tiny cores/fades.

If rounding is acceptable, compare source/full and exclusive halo/composite times with existing A/A controls. If the restricted eligible cases have no useful gain,
stop before generalizing alignment or creating a production pipeline policy.
