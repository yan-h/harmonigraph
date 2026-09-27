# Half-jitter four-neighbor trial

Measured on M1 Pro / Metal on 2026-09-27.
The baseline is the preceding four-neighbor build at `f4ce0de16f27fb2d6162dc04f07dde95972522ef`.
The candidate halves positional jitter to 0.3 cell widths and widens the halo window to 0.7–0.85 cells.
Both retain all five layers and the current defaults,
including color memory and the automatic full-resolution 2+3 split at 4K.

| Run | Full-jitter mean | Half-jitter mean | Half-jitter GPU time change | A/A difference |
| --- | ---: | ---: | ---: | ---: |
| 1080p A | 6.887 ms | 6.879 ms | -0.12% | -1.85% |
| 1080p B | 7.082 ms | 6.980 ms | -1.44% | -2.99% |
| 4K A | 19.755 ms | 20.197 ms | +2.24% | +0.05% |
| 4K B | 19.722 ms | 19.946 ms | +1.14% | -0.94% |

The wider halos cost about 1–2% at 4K relative to the preceding four-neighbor trial.
The 1080p differences are smaller than the variation between identical baseline controls,
so these runs do not establish a speed change there.
These are synthetic offscreen GPU intervals,
not live Bitwig frame times.
Do not combine their percentages with the earlier nine-neighbor comparisons across different runs.

The [preceding trial's harness and fixture](../four-neighbor-current/README.md) are reused unchanged:
two identical baseline cases and one candidate,
independent resources and memory,
all six execution orders,
60 warmup rounds and 240 measured rounds per case.
The baseline column averages the two baseline means;
positive A/A means baseline B took less time than A.
Raw samples,
logs and unrounded results are adjacent.
The `reduction_percent` field in `analysis.json` is the negative of the table's time-change column.

The renderer's 346 tests pass,
including the geometry bound for excluded cells and the split/native comparisons.
Five inspected Stars-bearing offline goldens change with the new positions and broader halos;
relative to the preceding trial their means move by 2.621–5.331/255 and maximum by 214/255.
Watercolor and lattice goldens remain unchanged.

## Reproduce

Start at `f4ce0de16f27fb2d6162dc04f07dde95972522ef` in an owner-managed worktree,
and copy this directory's `variant.patch` there from the reviewed revision.
Remove both scratch patches before building a plugin.

```sh
git show f4ce0de16f27fb2d6162dc04f07dde95972522ef:crates/harmonigraph-render/src/shaders/spectrogram.wgsl > /private/tmp/stars-four-baseline.wgsl
git apply --unidiff-zero variant.patch
git apply docs/evidence/spectrogram-stars/four-neighbor-current/harness.patch
cargo test --release -p harmonigraph-render --lib --no-run
python3 docs/evidence/spectrogram-stars/four-neighbor-current/run.py --binary /path/to/printed/test-executable --output /private/tmp/stars-half-repeat
git apply -R docs/evidence/spectrogram-stars/four-neighbor-current/harness.patch
git apply --unidiff-zero -R variant.patch
```
