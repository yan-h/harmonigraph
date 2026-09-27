# Four-neighbor full-jitter trial, current defaults

Measured on M1 Pro / Metal on 2026-09-27.
These measurements describe the full-jitter trial at `f4ce0de16f27fb2d6162dc04f07dde95972522ef`,
before the branch adopted half jitter and a wider halo window.
The baseline is `4b102f407f52e2ad00616fb7f1e7a3077334f929`.
Both variants retain the current defaults,
color memory and the automatic full-resolution 2+3 split at 4K.
The candidate changes only the five-layer walk to nearest 2x2 and its halo window to 0.6–0.7 cells.

| Run | Nine-neighbor mean | Four-neighbor mean | GPU time saved | A/A difference |
| --- | ---: | ---: | ---: | ---: |
| 1080p A | 9.469 ms | 6.105 ms | 35.5% | +0.34% |
| 1080p B | 9.420 ms | 5.997 ms | 36.3% | +2.09% |
| 4K A | 27.089 ms | 16.320 ms | 39.8% | -0.37% |
| 4K B | 26.751 ms | 16.165 ms | 39.6% | +0.07% |

Each run has two byte-identical baseline cases and one candidate,
with independent resources and color history.
All six execution orders repeat in balanced blocks after 60 warmup rounds,
followed by 240 measured rounds per case.
The repeat offsets the order schedule by three.
The baseline column averages the two baseline means;
positive A/A means baseline B took less time than A.
The 2.09% A/A discrepancy in the second 1080p run warrants caution about small effects,
but is much smaller than the candidate's difference.

This uses the existing synthetic noisy-grid fixture,
full pane coverage,
1024 slabs,
3828 buckets,
a ten-second history and two pixels per point.
Stars and memory evolve at the same clock in all three cases.
The timestamp interval runs from the first actual source pass through final composite completion.
These are synchronous offscreen GPU intervals,
not live Bitwig frame times or a recorded-project replay.
Raw samples are the adjacent compressed CSV files;
`analysis.json` retains unrounded means and ratios.

## Picture checks

The nearest-neighbor geometry and atlas-bound tests cover half-cell transitions,
fractional scale,
wrapped drift and fine-density layouts.
Existing split tests exercise retained color,
partial coverage,
extreme star controls and the actual large-pane cutoff.
At fractional scale the split/native comparison has maximum error 2/255 in one pixel's red and blue channels out of 92,480 channels;
all eight frames have mean error below 0.0018/255.
A scratch half-float round-trip in the native path did not remove that outlier,
so it cannot be attributed solely to intermediate texture quantization.
The regression now bounds maximum error at 2/255 and mean error at 0.01/255.

Five Stars-bearing offline goldens intentionally change:
tall and starfield mean 4.965/255,
short 3.950/255,
zoomed-in 5.329/255,
and mixed shadows 2.856/255.
Their contact sheets show the expected shorter halos and more separated points,
most pronounced when zoomed in.
The Watercolor golden is unchanged.
Visual acceptance remains for the live plugin trial.

## Reproduce

Use `f4ce0de16f27fb2d6162dc04f07dde95972522ef` in an owner-managed worktree.
Later branch revisions change positional jitter and do not reproduce this candidate.
The scratch harness is test-only and is removed before building a plugin.

```sh
git show 4b102f407f52e2ad00616fb7f1e7a3077334f929:crates/harmonigraph-render/src/shaders/spectrogram.wgsl > /private/tmp/stars-four-baseline.wgsl
git apply docs/evidence/spectrogram-stars/four-neighbor-current/harness.patch
cargo test --release -p harmonigraph-render --lib --no-run
python3 docs/evidence/spectrogram-stars/four-neighbor-current/run.py --binary /path/to/printed/test-executable --output /private/tmp/stars-four-repeat
git apply -R docs/evidence/spectrogram-stars/four-neighbor-current/harness.patch
```
