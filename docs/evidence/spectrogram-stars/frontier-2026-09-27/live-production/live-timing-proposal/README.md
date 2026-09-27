# Temporary production timing capture

The existing production `SOURCE_QUERY` hook already timestamps the beginning of the real source render pass.
No renderer or shader instrumentation is added.
The extracted test timestamps the end of the real final composite, exactly as the frozen ordinary two-query timing path does.
It asserts that the source actually consumed BEGIN; an accidentally narrowed paint-only interval fails.
No ACTIVE/SOURCE shader specialization, prototype resource selection, extra stage queries, or forced split mode remains.

This removable patch adds only `tests/live_timing.rs` and its module declaration.
It is independent of the temporary image parity module.
The generator extracts the established frozen harness and asserts each removed/replaced section, then checks patch applicability.
Its settings use the actual production enum and scalar.

- Cases: `half-a`, `half-b`, `full-a`, `full-b`, `p2`, `p3`.
  The A/A pairs use identical settings and independent resources.
- Default60 warmup rounds and240 measured rounds, all six cases each round.
- Six forward rotations and six reverse rotations repeat as a12-round schedule.
  Every case occupies every position exactly40 times in240 measured rounds.
- Recorded input keeps the established960 visible slabs in a1032-slot ring, one entering slab per frame, cyclic replay, clock1 + frame/48 seconds and default20-second history.
- Synthetic input keeps the established noisy grid and clock1 + frame/144 seconds, default10-second history.
- Default jitter0.5; other atmosphere settings are the current production defaults, matching frozen default research on this branch.
  Actual settings are printed for every case at the first frame.
- `PROBE_RAW` is required; CSV columns remain `frame,slot,case,gpu_ms,wall_ms`.
  `analyze_timings.py` recognizes both A/A pairs unchanged.
- Additional stage timing is deliberately excluded.
  The interval is GPU source-to-composite time, not end-to-end DAW latency.

## Apply and run (root agent only, after existing tests finish)

From the owned worktree:

```sh
/opt/homebrew/bin/python3 /private/tmp/stars-investigation/live-timing-proposal/make_patch.py
git apply /private/tmp/stars-investigation/live-timing-proposal/live-timing.patch
PROBE_INPUT=take PROBE_SIZE=1920x1080 PROBE_PPP=2 PROBE_JITTER=0.5 PROBE_FILLS=1 PROBE_WARMUP=60 PROBE_FRAMES=240 PROBE_RAW=/private/tmp/stars-investigation/live-take-1080.csv cargo test --release -p harmonigraph-render spectrogram::tests::live_timing::stars_live_production_timings -- --ignored --exact --nocapture --test-threads=1
/opt/homebrew/bin/python3 /private/tmp/stars-investigation/analyze_timings.py /private/tmp/stars-investigation/live-take-1080.csv
```

Use `PROBE_SIZE=3840x2160 PROBE_PPP=4` for the same960×540-point pane as the accepted4K image fixture.
Use PPP2 at4K only if matching an earlier timing run that used a1920×1080-point pane; star scale and workload differ.
Use `PROBE_INPUT=synthetic` for the existing synthetic fixture.
Keep all other controls equal between compared runs; inspect printed settings and source dimensions.
`RESEARCH_CASES` and `PROBE_STAGES` must be unset.
Optional `PROBE_ORDER_OFFSET` shifts the existing12-round order while preserving slot balance.
For a smoke run,12 measured rounds are accepted; timing evidence should use240.

Preserve the raw CSV, analysis JSON, log, production revision/diff, and this patch in the evidence bundle.
Judge P2/P3 savings against the mean of each A/A control pair, and compare the savings with A/A disagreement.
No performance conclusion is available until this production test is run in the quiet window.

Remove afterwards:

```sh
git apply -R /private/tmp/stars-investigation/live-timing-proposal/live-timing.patch
```

If subsequent edits or formatting changed context, remove only this temporary module and its declaration.
The existing permanent timing probe and its SOURCE_QUERY hook stay unchanged.
