# Split the dense Stars layers into two passes

Measured on Apple M1 Pro / Metal on 2026-09-26 against `3a3846fd`.
Yan preferred the full nine-neighbor appearance in the earlier comparison,
then asked to try rendering the two finest layers at 75% width and height.
This experiment keeps all five layers and their full halos.
No production rendering changes are enabled.

## Result

At 4K,
rendering the two farthest layers separately at native resolution reduced measured GPU time by 15.5–15.8%.
Reducing that intermediate to 75% width and height increased the saving to 23.1–25.0%,
with a visibly softer fine-grain layer.
At 1080p,
neither variant established a useful gain.
The full-resolution split is therefore a promising follow-up for retaining the preferred picture;
the reduced-resolution option still needs Yan's visual choice.

| Paired run | Current A | Full-resolution split | 75% split |
|---|---:|---:|---:|
| 1080p, 120 frames | 13.978 ms | 14.766 ms | 14.061 ms |
| 1080p, 240 frames, reversed order | 14.366 ms | 14.250 ms | 14.059 ms |
| 4K, 120 frames | 34.778 ms | 29.393 ms | 26.756 ms |
| 4K, 240 frames, reversed order | 35.564 ms | 29.952 ms | 26.673 ms |

The extra pass at 100% also improves performance at 4K,
so the entire 75% result cannot be attributed to drawing fewer pixels.
The reason the native split helps was not isolated;
compiler scheduling and resource use are possibilities rather than measured explanations.
These are synchronous offscreen GPU intervals,
not full DAW frame times.

## Implementation

`base` uses the existing shader plus an unused fragment entry point needed by the scratch pipeline builder.
It does not allocate an intermediate or draw an extra pass.
`dust100` and `dust075` have identical shader source;
the harness selects their intermediate dimensions.
Both draw layers 0 and 1 over the background into an RGBA16Float target,
then linearly sample that target underneath layers 2 through 4 in the final draw.
The original nine-neighbor walk,
life fade,
fringe and layer order remain intact.
The intermediate keeps the existing gamma-coded RGB compositing convention.
Both variants still run light preparation,
color memory and the full five-layer star-atlas bake.
The 75% intermediate has 56.25% as many pixels as the native target.

## Measurements and appearance

Cases use `SpectralAtmosphere::default()` with Stars selected,
including density 10,
color pickup/release 0.08/0.6 seconds,
and the approximately 4.19-million-cell atlas at this aspect ratio.
The timing fixture has full history coverage,
1024 slabs of 3828 buckets,
a ten-second span and two pixels per point.
Cases are interleaved after ten warmup rounds.
The table uses the median `end/full` values in the detailed min/p10/med/p90/max log rows.
Absolute times vary with shared-machine load;
compare candidates within each row.
No other experiment's rendering or video encoding ran during these timings.

- [1080p screening](1080.log) and [confirmation](1080-confirm.log)
- [4K screening](4k.log) and [confirmation](4k-confirm.log)

The images use native 1920×1080 rendering at one pixel per point,
the current default gradient,
a twenty-second recording-derived light field and two seconds of color-memory warmup.
The recording input comes from the look-prototype kit's `take-2026-09-11_03-16-17.wav` around the 72–92-second window.
A flat level of 128/255 provides a second control.
This is not a full saved Bitwig project or analyzer replay,
and the visual fixture differs from the synthetic timing fixture.
Six-second 60-fps clips compare A and the 75% split using the same evolving input and clock.
The full-resolution control was checked in the initial recording and flat frames only.

| Candidate | Recording: mean / max RGB error out of 255 | Flat: mean / max |
|---|---:|---:|
| Full-resolution split | 0.00331 / 1 | 0.00386 / 1 |
| 75% split | 1.05568 / 100 | 1.08727 / 75 |

The full-resolution difference is consistent with the intermediate's half-float quantization.
The reduced-resolution difference also resamples small bright peaks;
its low mean error is not a bound on the local change.
The inspected crops show softer fine grain with the larger stars retained.
Motion clips are provided for visual evaluation,
not proof that there is no additional shimmer.
These captures use an integer-aligned full pane;
fractional pane positioning,
other pixel ratios,
disabled memory,
control extremes and other GPUs remain unvalidated for a production implementation.

[Raw initial-frame metrics](image-metrics.json),
[motion-frame metrics](motion-frame-metrics.json),
and the [base](render-base.log),
[native split](render-dust100.log),
and [75% split](render-dust075.log) logs preserve the picture checks.
The media and recording-derived byte inputs are local experiment artifacts,
not repository assets.
No golden was re-baselined.

## Reproduce

Use an owner-managed worktree at the measured source plus these evidence files.
The preparation script verifies the shader hash and writes variants only to scratch.
The harness deliberately loads them from `/private/tmp/stars-round3`.
It is a temporary experimental pipeline,
not production code:
do not build or load the plugin with the harness applied.

```sh
python3 docs/evidence/spectrogram-stars/round3/prepare.py
git apply --check docs/evidence/spectrogram-stars/round3/harness.patch
git apply docs/evidence/spectrogram-stars/round3/harness.patch
HARMONIGRAPH_SHADER_ASSETS=source PROBE_COMPARE=base,dust100,dust075 \
  PROBE_CASE=stars PROBE_FILLS=1 PROBE_FRAMES=240 PROBE_SIZE=3840x2160 \
  cargo test --release -p harmonigraph-render cloud_costs_by_style_and_dial \
  -- --ignored --nocapture --test-threads=1
git apply -R docs/evidence/spectrogram-stars/round3/harness.patch
```

Set `PROBE_SIZE=1920x1080` for 1080p.
Reverse `PROBE_COMPARE` to reverse the case order.
The initial runs use 120 frames.
The image harness is retained in the patch;
its local byte inputs are unnecessary for reproducing the synthetic timings.
All temporary production-source changes were removed after the experiment.
