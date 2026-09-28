# Far fill slider validation

The production slider defaults to 0%, with the gentle prototype at 50% and the strong prototype at 100%.
The [prototype evidence](https://github.com/yan-h/harmonigraph/tree/e90cca23/docs/evidence/spectrogram-stars/crack-fill-2026-09-28) documents the appearance and coverage tradeoff.
These measurements validate its continuous production port rather than selecting a preferred appearance.

## Picture and behavior

Each of the three reference positions matches its prototype byte-for-byte over 144 frames at 1536×864, after 40 seconds of warmup on the same recorded take and appearance.
The 0% prototype also matches the original shader.
The capture harness rejects appearance parse warnings; an initial malformed scratch appearance was discarded before these captures.
See `capture_parity` in [validation.json](validation.json).

Existing tests were extended for persisted/defaulted/clamped fill, actual UI range, unchanged paused color history, Uniform split/unsplit rendering across five fill values, Optimized fractional partial regions, and silence over a nonblack background.
Local results: 19 Stars renderer tests, 52 persistence tests and 5 settings-range tests passed.

## Bounded GPU comparison

An interleaved scratch probe measured source-BEGIN to composite-END with density 10, memory enabled, 39.149967 seconds of history and 90.02282 semitones of span.
Each case used 120 warmup frames and 240 timed frames; 1080p used scale 2 and 4K used scale 4.
The same machine ran an initial screen and then a repeat in reverse resolution order.
No other task-owned GPU capture ran concurrently.
Absolute timings drifted, so they are not a cross-run speed comparison.

In `old_shader_control`, A and A2 use the untouched shader from main at `7f88775e`; B uses the production slider shader at 0%, and C at 100%.
A temporary shader-source hook selected the old source before constructing that case's pipelines; production source was restored after building the scratch probe.
The `paired_savings` values compare each frame to the mean of the two interleaved controls; negative values mean overhead.

| Run | 0% overhead | 100% overhead | A/A median drift |
| --- | ---: | ---: | ---: |
| 1080p | 0.26% | 0.32% | +0.10% |
| 4K | 0.43% | 0.61% | −0.59% |
| 4K repeat | 0.73% | 0.55% | −0.70% |
| 1080p repeat | 0.19% | 0.30% | −0.16% |

This supports a small sub-1% cost in these dense Optimized cases, not a universal zero-cost claim.
The earlier `new_shader_level_comparison` is retained too: its A/A2 are both the new shader at 0%, B is 50%, and C is 100%.
Its first 1080p run had 3.54% control drift; the direct old-shader comparison above was added because that run could not establish baseline overhead.
Uniform performance was not separately benchmarked.

## Metal corpus

The matching corpus was generated and validated by [Metal shader assets run 36381538916](https://github.com/yan-h/harmonigraph/actions/runs/36381538916), including strict catalog, renderer/offline goldens and fallback controls.
It is imported into the same commit as the shader change.
