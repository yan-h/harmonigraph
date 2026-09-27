# Stars halo cost probe — 2026-09-27

Production revision: f5d90b663f15d3eec1ff23c5282aeaa0c46e4339.
Apple M1 Pro, Metal, source-compiled shaders, full pane, default 50% Jitter and Fringe 0.5, five layers, retained color enabled.
The fixture matches the prior paired benchmark: 1024 slabs, 3828 buckets, a ten-second history and two pixels per point.

Each run interleaves full A, identical full B, and frozen halo updates in all six orders.
All cases render normally for 60 warmup frames.
For 180 measured frames the frozen case retains those halo images and skips only their five update draws.
Core evaluation, color memory, star baking, blur, halo sampling, native 2+3 split at 4K, and final composition still run.
The repeat shifts order by three.
Times are means of the actual first-source-begin to final-composite-end GPU interval.

| Run | Full ms | Frozen ms | Update difference ms | Reduction | A/A difference |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1080-a | 7.278 | 4.883 | 2.395 | 32.91% | -1.15% |
| 4k-a | 15.089 | 10.500 | 4.589 | 30.41% | -0.93% |
| 4k-b | 14.862 | 10.428 | 4.433 | 29.83% | -0.14% |
| 1080-b | 6.243 | 4.208 | 2.035 | 32.60% | -0.63% |

Generating the halo images therefore costs roughly 30% of the 4K render and 33% at 1080p in this fixture.
This is a marginal cost measurement, not an exclusive per-pass percentage: freezing changes the image and GPU scheduling, while leaving halo reads/composition in place.
Absolute times vary with concurrent load; the repeated paired ratios are the useful result.
The remaining core, composition and light-preparation costs have not been independently separated.

The initial stage probe is retained in `analysis.json`,
its compressed CSV files and `harness.patch`.
Its pass timestamps overlap: at 4K the apparent far and final pass durations each include waits for earlier fragment work, so summing durations exceeds the frame.
Do not treat them as exclusive costs.
This is consistent with the timing limitation already recorded in issue #1203.
The source-to-final intervals remain the valid comparison.

In that probe, Fringe zero did not consistently improve total time: changes ranged from about 0.4% slower to 1.1% faster and did not repeat consistently at either resolution.
Fringe zero still renders Gaussian tails in all halo layers; it is not a halo-pass bypass.

Both scratch patches were removed from production code after measurement.
This report records the historical `f5d90b66` baseline;
the branch now also has an adjustable halo resolution.

## Reproduce

The two patches are independent alternatives against the revision named above.
Apply `freeze-harness.patch` for the paired freeze comparison,
or `harness.patch` for the initial stage/Fringe probe;
do not apply them together.
In an owner-managed worktree at that revision,
apply the preserved patch and build the test executable.
For the freeze comparison,
use its path in the checkout containing this evidence:


```sh
git apply /path/to/evidence/halo-cost/freeze-harness.patch
HARMONIGRAPH_SHADER_ASSETS=source cargo test --release -p harmonigraph-render --lib --no-run
```

Run the printed executable with these environment variables and arguments:

```sh
HARMONIGRAPH_REQUIRE_GPU=1 HARMONIGRAPH_SHADER_ASSETS=source \
  PROBE_CASE=profile PROBE_FILLS=1 PROBE_FRAMES=180 \
  PROBE_SIZE=1920x1080 PROBE_ORDER_OFFSET=0 PROBE_RAW=/private/tmp/stars-halo-cost.csv \
  /path/to/test-executable cloud_costs_by_style_and_dial --ignored --nocapture --test-threads=1
```

Repeat at 3840x2160,
then repeat both sizes with order offset 3.
The paired analysis averages the two control means before computing the candidate's reduction.
The raw samples and summary JSON retain every run;
CSV files prefixed `freeze-` belong to the freeze comparison.
The other CSVs retain the stage probe,
whose full intervals support the Fringe comparison but whose overlapping stage durations must not be summed.
Reverse the applied patch before a production build:

```sh
git apply -R /path/to/evidence/halo-cost/freeze-harness.patch
```

For the stage/Fringe comparison,
substitute `harness.patch` in both apply commands.
