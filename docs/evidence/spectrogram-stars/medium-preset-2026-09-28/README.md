# Medium Stars preset production confirmation

The Stars rendering menu offers Medium,
High (the previous Optimized preset),
and Uniform.
High stays the default and retains the saved `P3` value.
Medium adds a saved enum variant;
older binaries that do not know it cannot parse appearances saved with Medium.
No enum is removed and no compatibility alias is introduced.

Medium uses 50% width and height for the back three layers,
75% for the foreground-over-back composite,
and 75%/45% for the two foreground halo targets.
The gamma-encoded intermediate is sampled through the existing texture binding.
Texture mix and its underlying spectrogram remain native,
so partial mix does not soften the base picture as the whole-composite research prototype did.
All five layers remain,
and the current star geometry and Far fill are shared by every preset.
This port includes the subsequently shipped equal-depth Star softness change.

## Cost

Apple M1 Pro / Metal;
current production source shaders;
1920×1080 at 2 px/pt and 3840×2160 at 4 px/pt;
240 measured frames after ten warmup frames.
The maintained probe interleaves High,
Medium,
and High with color memory every frame.
Only the two memory-off cases are compared below.
These are source-pass BEGIN to dependent final-pass END intervals,
not whole-DAW frame times.
No local builds or captures ran concurrently.

| Size | High run medians | Medium run medians | Reduction within each run |
| --- | --- | --- | --- |
| 1080p | 5.171, 5.744 ms | 4.051, 4.576 ms | 21.7%, 20.3% |
| 4K | 14.847, 14.853 ms | 9.144, 9.154 ms | 38.4%, 38.4% |

The 1080p runs have noticeable background-load noise,
so use an approximate 20–22% saving rather than a precise frame-time promise.
The 4K repeat is stable.
High's 4K cost remains close to the earlier current-profile measurements,
but this is not a controlled old-binary/new-binary regression benchmark.

Run the maintained `cloud_costs_by_style_and_dial` probe with `PROBE_CASE=stars`,
`PROBE_FILLS=1`,
`PROBE_FRAMES=240`,
and the size/PPP combinations above.
`timing.json` records binary provenance and all percentile summaries;
the logs retain the complete output.

## Verification

The 73 spectrogram regressions pass.
The extended fractional-pane fixture tests both High and Medium,
gamma and sRGB outputs,
partial Texture mix,
jitter extremes,
and retained colors.
It asserts actual far/near dimensions and that Medium encodes exactly one extra pass compared with High.
The clipping comparison includes the first and last covered pixel centers;
it pads the near footprint first and then maps that footprint to the far target,
so the two bilinear sampling stages both have valid boundary taps.

Color-history tests carry lit history across High/Medium/Uniform allocations while the current input is dark.
Persistence tests round-trip every preset through editor and offline appearance state.
Medium is included in the real settings-bar range scenarios.
The new `spectrogram-starfield-medium` export golden has been inspected alongside High:
the same structure and palette remain,
with softer fine grain.
Existing offline goldens pass unchanged.

The prior [resolution report](../resolution-followup-2026-09-28/README.md) retains the full experimental visual metrics and comparison crops.
Those numbers describe its recorded-derived test field and research composite;
they are not substituted for this port's golden or claimed as byte-exact port parity.

The [Metal regeneration run](https://github.com/yan-h/harmonigraph/actions/runs/36433647516) passed strict catalog,
renderer/offline goldens,
and binding/fallback controls.
Its generated corpus is included in the same implementation commit.
Local strict Clippy,
persistence,
settings ranges,
and fourteen lattice golden checks also passed.
