# Full-resolution Stars halos versus gathered blur

Measured on M1 Pro / Metal on 2026-09-27,
against `2f8b7bbdd3cd16009645f23d8690151e1a623e55` in draft PR #1234.
This follows the [gathered-blur experiment](../gathered-blur/README.md).
The previous comparison used one-third analytic resolution;
it did not answer the user's revised quality target:

> I do think 1/3 resolution blur looks pretty bad.
> I think 1/2 blur is minimum,
> and full resolution looks slightly better.
> so if something can match full resolution for significantly cheaper,
> I'd consider it

Here “resolution blur” refers to the analytic Halo resolution slider.
**Full-resolution analytic halos are now the visual reference;
half resolution is the user's minimum acceptable sampling compromise.** The prototype has not been accepted as matching that reference.
Production code,
controls and defaults remain unchanged;
only the comparison evidence is committed.

## What was compared

- **A: analytic halos at 100%.** All five halo images render at native resolution.
- **B: analytic halos at 50%.** The existing minimum acceptable compromise,
included in the pictures and final timing series.
- **C: gathered blur.** The preserved original seven-tap-per-axis prototype,
with each depth's internal logical resolution capped at exactly one third of the pane.

All variants use 100% jitter,
five layers,
color memory and the same native cores.
The candidate's resolution was not raised to 100%:
that would change the candidate instead of comparing the saved cheaper construction against the better reference.
The seven-tap version avoids additional kernel truncation from the later trimmed variants.
Its earlier paired penalty was smaller,
but changing load across those series did not establish a controlled ranking among filter variants.

## Pictures and motion

These are actual GPU renders at 1920×1080,
with a 960×540-point pane and two device pixels per point.
They use the same light field derived from the private take `take-2026-09-11_03-16-17.wav`,
72–92 seconds,
and the same palette as the earlier experiment.
A constant-level field checks the texture independently of musical structure.
The crops enlarge a 400×300-pixel region by exactly 2× with nearest-neighbor replication.

- [Real-take 2× comparison](full-halo-take-2x.png)
- [Flat-field 2× comparison](full-halo-flat-2x.png)

A retains more distinct rounded halo shapes.
B softens those shapes but follows them more closely than C in these crops.
C produces a more even granular glow and changes the fine halo structure.
The difference is clearer against full resolution than it was against one third.
That is a visual assessment of these fixtures,
not a declaration of the user's preference.

On the take,
mean absolute RGB difference from A is 1.431/255 for B and 2.494/255 for C.
On the flat field it is 1.803/255 and 3.069/255 respectively.
Average brightness remains close;
that does not imply equal shapes or equal perceived quality.
[The image metrics](image-metrics.json) preserve both mean colors and pixel differences.

A three-second side-by-side clip shows A/B/C as native 640×480-pixel crops from the 1080p renders,
with labels above them.
It contains 90 frames at 30 fps,
advancing the Stars clock from 3 seconds while keeping the musical light field static.
The clip was shown in the chat;
[motion.py](motion.py) reproduces the labeled assembly from the GPU harness's three videos.
The compressed clip is for motion judgment;
the PNGs retain the exact rendered pixels.
Sampled first,
middle and last frames preserve the same qualitative difference.
The independent GPU seed-conservation test passed again;
[the log](seed-check.log) records that check.
It does not prove equality to the analytic halo response.

## Paired timings

The final series interleaves two independent full-resolution controls,
the half-resolution analytic case and the gathered-blur case.
It uses 60 warmup rounds and 240 measured rounds per case,
cycling all 24 permutations of the four cases.
Each measured series therefore has ten complete permutation cycles;
the repeat offsets the schedule by three.
All renders use source-compiled shaders.
The full-resolution column averages the two controls;
positive A/A means control B ran faster than control A.

| Run | Full halos | Half halos | Gathered blur | Half saving vs full | Blur saving vs full | A/A difference |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1080-a | 9.316 ms | 6.642 ms | 6.958 ms | 28.71% | 25.31% | -0.76% |
| 4k-a | 35.761 ms | 19.372 ms | 17.377 ms | 45.83% | 51.41% | -0.34% |
| 4k-b | 32.220 ms | 19.076 ms | 15.263 ms | 40.80% | 52.63% | -1.70% |
| 1080-b | 13.049 ms | 8.920 ms | 9.611 ms | 31.64% | 26.35% | -1.25% |

The fixture is the synthetic noisy full-pane field with 1024 slabs,
3828 buckets,
a ten-second history and two pixels per point.
The automatic native 2+3 split remains enabled where the existing renderer selects it.
GPU timestamps cover the actual first source pass through final composite completion.
These are synchronous offscreen GPU intervals,
not live Bitwig frame times or guaranteed savings on other devices.
Use within-run ratios;
absolute times shift with background load and clocks.
No exclusive pass cost is inferred from overlapping Metal timestamps.

The preliminary three-case series omitted B and compared only two full-resolution controls with C,
cycling all six permutations.
It found 23.88% and 9.09% lower total GPU time at 1080p,
and 51.64% and 51.82% lower at 4K.
That wider 1080p variation is retained rather than hidden by the final series.
Short CPU video assembly overlapped the beginning of the preliminary series;
the final series started after builds and video encoding completed.
Other system load was not controlled.
The comparison demonstrates a cost opportunity at full resolution,
not visual equivalence or a stable percentage across every load condition.

Raw samples and logs are preserved under [the preliminary analysis](timings/analysis.json) and [the final four-case analysis](timings-with-half/analysis.json).
The reused runner calls gathered blur `paired four` and the half-resolution reference `paired half halos`.
[comparison-summary.json](comparison-summary.json) additionally derives both candidates' reductions and blur's saving relative to half-resolution halos.

## Decision and limitations

The prototype was removed from the active renderer after collecting the comparison.
A speedup against full-resolution analytic halos alone does not establish that it meets the user's request to match that appearance.
The user has not yet selected C from these comparisons.
Half-resolution analytic halos remain the accepted compromise;
full resolution remains the quality target.

The saved blur still uses a common Gaussian kernel per depth and approximate per-star mass,
so it cannot reproduce every star's analytic halo profile.
Its previously documented atlas-padding bound and repeated CPU border quadrature remain unresolved scratch-code limitations.
No new production selector,
persisted field,
shader asset or golden baseline is included.
If the user accepts the visual tradeoff,
those limits still need resolving before a supported plugin option.

## Reproduce

Use an owner-managed worktree at the baseline above.
Apply the preserved gathered-blur prototype,
restore its full seven-tap filters,
then apply one comparison harness patch:

```sh
git apply docs/evidence/spectrogram-stars/gathered-blur/prototype.patch
git apply docs/evidence/spectrogram-stars/gathered-blur/full-filter.patch
git apply docs/evidence/spectrogram-stars/full-halo-comparison/comparison-with-half.patch
HARMONIGRAPH_SHADER_ASSETS=source cargo test --release -p harmonigraph-render --lib --no-run
python3 docs/evidence/spectrogram-stars/four-neighbor-current/run.py --binary /path/to/printed/test-executable --output /private/tmp/stars-full-repeat
```

`comparison-with-half.patch` reproduces the final four-case timings and A/B/C pictures.
`comparison.patch` is the alternative preliminary three-case timing harness;
it produces the same A/B/C pictures.
Do not apply both comparison patches together.
Reverse the selected comparison patch,
then the full-filter patch,
then the original prototype when finished.
These are scratch source-shader experiments and do not regenerate the production Metal corpus.

Prepare the existing take and flat inputs with the [earlier input script](../gathered-blur/prepare-input.py),
then copy `take-levels.u8`,
`flat-levels.u8` and `palette.rgba` from `/private/tmp/stars-gather-blur` into `/private/tmp/stars-full-halo-compare`.
Run the ignored `scratch_take_stars_frames` test with `HARMONIGRAPH_REQUIRE_GPU=1`,
`HARMONIGRAPH_SHADER_ASSETS=source`,
`PROBE_JITTER=1` and `PROBE_IMAGE_PPP=2`.
Run once without `PROBE_VIDEO_FRAMES` to include the flat field,
then with `PROBE_VIDEO_FRAMES=90` to create the three motion videos.
The GPU harness requires `ffmpeg` for the motion run.

Run [sheet.py](sheet.py) with `--repo /path/to/repo --output /path/to/sheets` to assemble the native-size and 2× comparison sheets.
Run [motion.py](motion.py) with `--repo /path/to/repo --output /path/to/comparison.mp4` to assemble the native cropped motion clip.
Both scripts default their inputs to the scratch directory above.
