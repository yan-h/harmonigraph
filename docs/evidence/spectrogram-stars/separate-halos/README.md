# One-neighbor cores with separate halos

Measured on M1 Pro / Metal on 2026-09-27.
Baseline: configurable four-neighbor Stars at `4a35083afe12753a9e9264c4a37a2cd9565a867d`.
Candidate: the one-core shader at `ff02b47eca4a4e59a235fcf13cb88a5433d3d6ca`,
before its matching Metal corpus and these records were amended into the commit.
Both use default 50% jitter,
all five layers,
color memory and the automatic native 2+3 split at 4K.
The candidate restores the original wide halo response through five half-resolution images.

| Run | Four-neighbor mean | One core + halos mean | GPU cost change | A/A difference |
| --- | ---: | ---: | ---: | ---: |
| 1080-a | 5.533 ms | 5.756 ms | +4.04% | +0.31% |
| 4k-a | 14.625 ms | 13.417 ms | -8.26% | -0.04% |
| 4k-b | 14.632 ms | 13.395 ms | -8.45% | -0.46% |
| 1080-b | 5.540 ms | 5.752 ms | +3.83% | -0.39% |

The prototype saves 8.3–8.5% at 4K but costs 3.8–4.0% more at 1080p.
The added halo passes and bandwidth matter:
16.25 effective atlas candidates versus 20 is not a proportional timing prediction.
Halo storage adds 79.1 MiB at 4K,
plus the existing 63.3 MiB split target.
This is a visual/performance tradeoff,
not a speed improvement at every pane size.

Each run interleaves two byte-identical baseline cases and one candidate,
with independent resources and color history.
All six case orders repeat in balanced blocks after 60 warmup rounds,
followed by 240 measured rounds per case.
The second run offsets the order schedule by three.
Positive A/A means baseline B ran faster than A.
The baseline column averages both control means.
Raw samples retain the earlier runner's candidate name `paired four`;
here that case is the one-core-plus-halos candidate.

The fixture is the existing synthetic noisy grid,
full pane coverage,
1024 slabs,
3828 buckets,
a ten-second history and two pixels per point.
GPU timestamps span the actual first source pass through final composite completion,
including every new halo pass.
These are synchronous offscreen intervals,
not Bitwig frame times or a recorded-project replay.
The scratch baseline skips halo draws and restores the prior CPU geometry values,
so it runs the prior shader unchanged except for an unused stub required by the new pipeline constructor.
Both variants allocate the new halo array,
so these timings do not compare allocation or peak-memory cost.

## Picture and correctness

One-cell core and three-cell halo bounds cover Jitter 0–100%.
The atlas-bound test replays the fractional-coordinate path around cell boundaries and drift wraps.
At native halo resolution,
the independent original-response comparison differs by at most 1/255 and mean below 0.03/255 across Jitter 0%,
50% and 100%,
with Fringe zero and maximum.
A core-only witness requires visible halos in every case,
including Fringe zero's Gaussian tails.
Split parity exercises fractional display scale,
partial regions,
color memory,
extreme controls and the actual large-pane threshold.

An initial reconstruction probe found up to 6/255 error at full jitter,
even with native halo sampling.
Exact texture loads and a halo-only probe ruled out the sampler as the cause.
Splitting wrapped drift into integer and fractional pieces before distance evaluation removed the discrepancy.
The smaller coordinates now agree across core and halo pipelines.

Five offline goldens intentionally change from the four-neighbor trial:
short pane mean 8.379/255,
tall and starfield 5.734/255,
zoomed-in 3.558/255,
and mixed spectral shadows 4.865/255.
The inspected images show wider,
smoother glow and less fine grain,
most obvious in small dense panes.
Watercolor and lattice frames remain unchanged.
The wide response is reconstructed at half resolution in production,
so the native reconstruction check does not claim pixel identity for the shipped look.
Visual acceptance remains for the plugin trial.

## Reproduce

Use this revision in an owner-managed worktree.
The scratch patch is test-only and must be removed before a plugin build.
The renderer is compiled from source for timings;
no generated Metal corpus is used for either variant.

```sh
git show 4a35083afe12753a9e9264c4a37a2cd9565a867d:crates/harmonigraph-render/src/shaders/spectrogram.wgsl > /private/tmp/stars-four-baseline.wgsl
python3 - <<'PYCODE'
from pathlib import Path
p = Path('/private/tmp/stars-four-baseline.wgsl')
p.write_text(p.read_text() + '\n@fragment fn fs_star_halo() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }\n')
PYCODE
git apply docs/evidence/spectrogram-stars/separate-halos/harness.patch
cargo test --release -p harmonigraph-render --lib --no-run
python3 docs/evidence/spectrogram-stars/four-neighbor-current/run.py --binary /path/to/printed/test-executable --output /private/tmp/stars-halo-repeat
git apply -R docs/evidence/spectrogram-stars/separate-halos/harness.patch
```
