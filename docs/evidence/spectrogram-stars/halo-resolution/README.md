# Halo resolution: 50% versus one third

Paired M1 Pro / Metal measurements on 2026-09-27,
using the Halo resolution control at `e0000925d3ddfdbb206d4269731227d4dbf5a87f` before its generated Metal corpus and this evidence were amended into the same commit.
All five depth layers,
50% jitter,
color memory and the automatic native 2+3 split remain enabled.
Both cases use the analytic halos with their full 1.2-cell reach.
Only the halo image resolution changes.

| Run | 50% mean | One-third mean | GPU time reduction | A/A difference |
| --- | ---: | ---: | ---: | ---: |
| 1080-a | 7.542 ms | 6.888 ms | 8.67% | -1.08% |
| 4k-a | 20.041 ms | 16.457 ms | 17.88% | +0.28% |
| 4k-b | 19.487 ms | 16.061 ms | 17.58% | +0.62% |
| 1080-b | 7.562 ms | 7.019 ms | 7.18% | +0.63% |

One-third resolution saves 17.6–17.9% of total GPU time at 4K and 7.2–8.7% at 1080p in these runs.
It evaluates approximately 56% fewer halo pixels;
the native cores,
color processing and composition remain.
The default stays at 50% so the sampling tradeoff is a user choice.
The exact measured setting is `1.0 / 3.0` rather than `0.33`;
the UI rounds its displayed percentage.

Each run interleaves two identical 50% controls and one candidate,
with independent resources and color history,
60 warmup rounds and 240 measured rounds per case.
All six orders repeat in balanced blocks;
the repeat offsets that schedule by three.
Positive A/A means control B ran faster than A.
The first column averages the controls.
Raw CSVs retain the shared runner's `paired four` name for the one-third candidate.

The synthetic fixture has full pane coverage,
1024 slabs,
3828 buckets,
a ten-second history and two device pixels per point.
Timestamps span the actual first source pass through final composite completion.
These synchronous offscreen measurements are not live Bitwig frame times.
Compare paired ratios,
not absolute times from earlier investigations or other background loads.
No exclusive pass costs are inferred from overlapping Metal pass timestamps.

## Reproduce

Apply the scratch timing patch to this revision in an owner-managed worktree.
It changes the timing test only and must be removed before handoff.
The runner uses shader source for both cases.

```sh
git apply docs/evidence/spectrogram-stars/halo-resolution/harness.patch
HARMONIGRAPH_SHADER_ASSETS=source cargo test --release -p harmonigraph-render --lib --no-run
python3 docs/evidence/spectrogram-stars/four-neighbor-current/run.py --binary /path/to/printed/test-executable --output /private/tmp/stars-resolution-repeat
git apply -R docs/evidence/spectrogram-stars/halo-resolution/harness.patch
```

[analysis.json](analysis.json) contains the means and ratios;
the four compressed CSVs preserve the samples.
