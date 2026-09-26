# Longer Stars layer-split comparisons

Follow-up to the initial [split-layer experiment](../README.md),
measured on the same M1 Pro / Metal system and source `3a3846fd`.
The question is whether the apparent 4K gain survives longer sampling,
execution-order changes and a second input workload.
No production rendering changes are enabled.

## Results

Eight runs provide 19,200 measured case samples after warmup.
The native split saves 14.6–17.3% of measured GPU time in all four 4K runs,
across both workloads and substantial changes in absolute performance.
Every one of the 100 balanced 4K blocks favors the native split.
Its submit-to-completion wall-time saving is 12.9–17.0%,
which supports the GPU-timestamp finding without establishing a full DAW frame-time benefit.
The 75% split saves 23.2–26.3% at 4K,
with the previously documented change in fine grain.

At 1080p,
the native split has no dependable gain.
The first recording run reports 5.1% higher mean GPU time,
but its repeat is nearly tied at 0.9% higher;
the synthetic runs are also nearly tied.
These results do not establish a consistent 5% regression,
but they do not justify switching the 1080p renderer to the split path.
The 75% split saves 1.0–3.8% in the 1080p runs,
with the weakest repeat's interval crossing zero.

Positive percentages below mean **less time**;
negative percentages mean more time.
A/A compares baseline copy B with baseline copy A.
The split columns compare against the mean of both baseline copies.
The block ranges are descriptive within-run bootstrap intervals,
not guarantees or calibrated confidence bounds for product performance.

| Workload | Native split | 75% split | A/A difference | Native 95% block range |
|---|---:|---:|---:|---:|
| [4K synthetic, seed 17](synthetic-4k-17.csv.gz) | +14.6% | +23.2% | +1.0% | +13.3% to +15.9% |
| [4K synthetic, seed 53](synthetic-4k-53.csv.gz) | +16.1% | +24.1% | -0.2% | +15.3% to +16.9% |
| [4K recording, seed 29](recording-4k-29.csv.gz) | +15.8% | +24.7% | -0.3% | +14.3% to +16.9% |
| [4K recording, seed 67](recording-4k-67.csv.gz) | +17.3% | +26.3% | -1.9% | +15.7% to +19.0% |
| [1080p synthetic, seed 41](synthetic-1080-41.csv.gz) | +0.7% | +3.3% | -0.3% | -0.9% to +2.1% |
| [1080p synthetic, seed 97](synthetic-1080-97.csv.gz) | -0.5% | +1.0% | +0.6% | -1.6% to +0.6% |
| [1080p recording, seed 79](recording-1080-79.csv.gz) | -5.1% | +3.3% | +1.3% | -8.9% to -1.8% |
| [1080p recording, seed 83](recording-1080-83.csv.gz) | -0.9% | +3.8% | -0.3% | -2.2% to +0.3% |

The duplicate baselines show apparent differences as large as 1.9%,
and two of their descriptive ranges exclude zero.
This is a warning against treating these block-bootstrap intervals as formal significance tests.
The 4K gain is much larger than that observed A/A discrepancy and repeats in both workloads;
the small 1080p effects deserve more caution.
Absolute baseline medians range from approximately 33 to 48 ms across the 4K runs.
The cause of that variation was not isolated.

[Full analysis](analysis.json) retains medians,
means,
p10/p90 spreads,
paired median differences,
block results and both timestamp/wall-time cross-checks.
The native split remains a justified 4K optimization candidate,
with a baseline path still appropriate at 1080p.
Intermediate pane sizes have not been measured,
so these endpoints do not establish a resolution threshold.
Appearance checks outside the aligned preview and live DAW validation remain outstanding before shipping.
No build was loaded into the DAW.

## Method

Each process runs four cases:
`base_a`,
`base_b`,
`dust100` and `dust075`.
The two baseline files are byte-identical,
with SHA-256 `429c009944e56dc4add07502c9f37d36fbf448f2cbf4283766fa57fd7f64b860`.
They own independent persistent resources,
as do the two split variants.
The split shaders and intermediate formats are unchanged from the initial experiment.

Every process discards 120 warmup rounds,
then measures 600 rounds.
Each round executes all four cases at the same virtual time and input state.
Every group of 24 rounds uses every permutation of the four cases once,
in a seeded shuffled order.
Thus each case occupies every execution slot exactly 150 times in the measured region.
The cases are submitted and waited for sequentially;
there are no simultaneous benchmark or video-rendering jobs.
The desktop is not an isolated GPU environment,
and clocks and thermals were not pinned or recorded.

Two workloads are used:

- The original synthetic full-coverage fixture has 1024 slabs of 3828 bins,
  a ten-second span,
  two pixels per point and a 144-Hz virtual clock.
  Its history is static and has no steady-state grid uploads.
- The recording-derived fixture has 961 visible slabs of 1024 bins,
  a twenty-second span,
  one pixel per point and a 60-Hz virtual clock.
  It uses the preview's light input and current default gradient.
  Shared slab allocations and monotonically increasing keys retain history while 48 new columns per virtual second enter the ring.
  The 1344-column source is repeated cyclically when exhausted;
  this is a repeating recording-derived light input rather than a saved DAW or analyzer replay.

Both use the current dense Stars defaults and full-pane coverage.
An assertion verifies exactly one initial full-history upload per case,
with scrolling handled by partial writes after that.
The recording quad spans 960 slab intervals while the fixture's `points_per_slab` uses a divisor of 961;
this approximately 0.1% geometry discrepancy is common to all candidates and differs from the preview fixture.

The CSV stores round,
execution slot,
case,
both full GPU timestamp brackets,
submit-to-completion wall time and CPU prepare time.
The primary interval is the opening timestamp pass's **begin** to the composite pass's **end**.
The opening pass's end-to-composite-end interval is retained as a cross-check and matches the earlier report's metric.
The opening pass is independent,
so its timestamps do not constitute a dependency barrier for every preparation operation.
The historical paint-begin-to-tail-begin interval can reverse on this GPU and is not used.
The harness asserts ordered opening begin/end and composite end timestamps for every measured sample.
Buffer uploads issued through `queue.write_buffer` can execute before the opening stamp;
these are GPU rendering intervals,
not complete streaming costs or DAW frame times.

The analysis compares each split against the mean of both baseline samples from the same round.
It also compares the two baseline copies against each other as a null control.
Savings are ratios of mean durations,
with individual medians retained separately.
For uncertainty,
24 consecutive rounds form one balanced block,
and 10,000 bootstrap resamples of the 25 blocks produce a descriptive 95% interval.
These intervals account for within-block correlation,
input or thermal effects can remain correlated across block boundaries,
and these intervals do not cover all variation across machines and applications.
The repeated 4K processes provide a separate repeatability check.

## Reproduce and inspect

The [compressed raw CSV files and environment records](.) retain every measured sample and invocation.
Compression keeps the evidence diff focused on the human-readable report.
The analysis accepts both the stored `.csv.gz` files and the runner's plain `.csv` outputs.
[Input hashes](input-sha256.json) identify the shader variants and recording bytes.
The recording-derived input remains a local preview artifact rather than a repository asset.
The synthetic runs need no external input.

Apply this directory's complete [scratch harness](harness.patch) to the measured production source;
do not also apply the earlier round-three harness.
The extra pipeline entry point is experimental,
so do not build or load a plugin with this patch applied.

```sh
python3 docs/evidence/spectrogram-stars/round3/prepare.py
cp /private/tmp/stars-round3/base.wgsl /private/tmp/stars-round3/base_a.wgsl
cp /private/tmp/stars-round3/base.wgsl /private/tmp/stars-round3/base_b.wgsl
git apply --check docs/evidence/spectrogram-stars/round3/confidence/harness.patch
git apply docs/evidence/spectrogram-stars/round3/confidence/harness.patch
python3 docs/evidence/spectrogram-stars/round3/confidence/run.py
python3 docs/evidence/spectrogram-stars/round3/confidence/analyze.py --synthetic-only
git apply -R docs/evidence/spectrogram-stars/round3/confidence/harness.patch
```

For all eight runs,
pass `--recording /path/to/take-levels.u8` to `run.py`,
and omit `--synthetic-only` from the analysis.
`run.py --binary /path/to/harmonigraph_render-test-executable` uses a previously compiled release test binary,
as the reported measurements did.
The default runner invokes the same ignored release test through Cargo.
`analyze.py` needs NumPy and accepts the directory containing the CSVs as its positional argument.
The analysis asserts the expected run set,
four distinct cases and execution slots per round,
equal slot occupancy and all 24 orders within every block.
All temporary production-source edits were removed after compiling the experiment.
