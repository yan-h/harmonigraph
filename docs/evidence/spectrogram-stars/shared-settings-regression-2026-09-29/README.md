# Stars Medium regression introduced by #1269

The shared Stars refactor in [#1269](https://github.com/yan-h/harmonigraph/pull/1269) increases this full-4K Medium workload from approximately 10 ms to 48–59 ms on Apple M1 Pro / Metal.
A temporary spectrogram-only probe on current main restores approximately 10 ms by replacing the shared settings-struct return with direct uniform reads.
The shipped precompiled Metal assets reproduce the regression with source fallback disabled.
No production fix is included in this report.
[Issue #1282](https://github.com/yan-h/harmonigraph/issues/1282) tracks the confirmed regression and remaining fix.

## Results

Each value is the GPU median of a separate 240-frame run after ten warmup frames.
The interval starts on the real source pass and ends on its dependent final composite.
These are offscreen renderer intervals for a complete 3840×2160 spectrogram,
not live DAW frame times or a prediction of the user's pane FPS.

| Comparison | Run order | Before/control medians, ms | After/candidate medians, ms |
| --- | --- | --- | --- |
| Before #1269 versus current main, source shaders | A A B B A B B A | 10.098, 10.003, 9.851, 9.332 | 58.912, 53.953, 53.819, 48.997 |
| Immediate parent versus #1269, source shaders | A B B A | 9.476, 9.341 | 48.422, 48.644 |
| Current main versus direct-access diagnostic, source shaders | A B B A | 52.074, 54.019 | 9.958, 10.092 |
| Before #1269 versus current main, shipped assets in strict mode | A B B A | 9.962, 9.910 | 56.415, 56.032 |

The strict-asset comparison is about 5.7× slower on current main.
CPU callback preparation stays around 0.13–0.15 ms;
queue submission through completion also grows with the GPU interval.
The repeated identical binaries and return-to-baseline runs rule out ordinary run variation as an explanation for the large separation.
No run was excluded.

## What changed

`star_settings()` in the spectrogram shader returns a complete `StarUniforms` by value,
including both five-element arrays.
The shared shader calls it repeatedly inside the per-fragment depth loop and halo calculations.
The generated Metal source retains those aggregate constructions where the old shader directly accessed bound uniform fields.

The diagnostic replaces all 36 occurrences of `star_settings().` with `cloud.` in a private copy of the shared shader,
and uses that copy only in the spectrogram consumer.
It changes no rendering algorithm,
resolution,
pass count,
color-memory setting,
or star geometry.
The lattice consumer keeps the unmodified shared shader.
[apply-direct-probe.py](apply-direct-probe.py) reproduces the temporary change.
All production source edits were restored after saving the diagnostic executable.

This isolates the aggregate-return adapter as the performance cause in the measured compiler/backend path.
It does not determine whether register pressure,
array materialization,
or another compiler lowering detail accounts for the extra cost.
The diagnostic is not a production fix:
image parity and the other consumer have not been validated for a replacement API.
A fix should preserve the shared Stars algorithm while avoiding the aggregate return in fragment hot paths,
then verify both consumers and regenerate the Metal corpus.

## Fixture and limits

- Apple M1 Pro, macOS 27.0, Metal; release builds with the same locked dependencies.
- 3840×2160 physical pixels at 4 pixels per point; full coverage; 1024 visible slabs in a 1032-slot ring; 3828 pitch buckets; 96-semitone span; 10-second history.
- `PROBE_CASE=medium` selects only the explicit Medium case. The parser treats commas as OR separators, so `PROBE_CASE='stars, medium'` would select all Stars cases.
- Medium is explicit on every ref; all Stars shape/density defaults match; color memory is disabled. No saved state is loaded, so the settings-format reset cannot explain this measured difference.
- The contours default is 17 before #1269 and 16 on current main. Stars bypass contour coloring at full texture depth; the immediate-parent/#1269 pair has identical defaults, and the current/direct-access pair also has identical defaults.
- Playback was stopped and the plugin editor closed. Builds completed before measurement; no build or other agent GPU benchmark ran concurrently. The machine remained a normal desktop.
- The live report was approximately 60 fps with spectrogram Stars on Medium, versus a previous 120+ fps. This experiment establishes a renderer regression, not the exact share of that live slowdown or its settings.
- No live DAW settings, plugin slot, or offline-renderer slot were changed. The experiment does not cover color memory, other profiles, other GPUs, or picture parity.

## Evidence and replay

[manifest.json](manifest.json) pins each source commit and executable SHA-256.
[results-3840x2160.json](results-3840x2160.json),
[results-refactor-3840x2160.json](results-refactor-3840x2160.json),
[results-direct-3840x2160.json](results-direct-3840x2160.json),
and [results-strict-3840x2160.json](results-strict-3840x2160.json) contain the run-level summaries.
The adjacent logs preserve complete probe output including adapter identity,
GPU min/p10/median/p90/max,
wall median,
CPU median,
and test status.
The maintained probe emits quantiles rather than individual frame samples;
these files do not contain per-frame raw timings.

Build the renderer test executable separately at the pinned `before`,
`refactor`,
and `current` commits in an owner-managed worktree:

```sh
cargo test --locked --release -p harmonigraph-render cloud_costs_by_style_and_dial \
  --no-run --message-format=json
```

Copy the reported `harmonigraph_render` test executable into an external evidence directory under its corresponding label.
Copy [run.py](run.py) and [apply-direct-probe.py](apply-direct-probe.py) there before switching revisions.
For `direct`,
run `apply-direct-probe.py` from the worktree at the pinned current commit,
rebuild,
and copy the executable as `direct`.
Restore `spectrogram.rs` and remove only the generated `stars-direct-probe.wgsl` before ending the experiment.
Do not install a diagnostic binary as a production plugin.

From the external directory containing those executables and the runner:

```sh
python3 run.py
BENCH_TAG=refactor BENCH_ORDER=before,refactor,refactor,before python3 run.py
BENCH_TAG=direct BENCH_ORDER=current,direct,direct,current python3 run.py
BENCH_TAG=strict BENCH_ASSETS=strict BENCH_ORDER=before,current,current,before python3 run.py
```

The runner defaults to source shaders for the first three comparisons.
Strict mode uses the compiled-in production assets and fails rather than falling back to source.
Results depend on compiler,
OS,
and GPU;
retain new logs rather than treating the values above as a universal timing threshold.
