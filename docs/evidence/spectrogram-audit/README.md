# Spectrogram audit evidence, 2026-09-05

Scope owner:
[spectrogram rendering plan](../../spectrogram-rendering-plan.md).
Source baseline:
`70e9f48dddb3c14e5cb2d2173d5604f8a4c34e84`.
Machine:
Apple M1 Pro, 8 CPU cores, 16 GB;
Rust 1.92.0;
release profile, one test thread.
No shared DAW slot was changed and no in-host measurement was taken.
The temporary probe changes only test visibility and adds ignored test code;
it implements no proposed rendering change.

## Files and commands

| Artifact | Meaning |
|---|---|
| [existing-ui.txt](existing-ui.txt) | Existing UI-only picture probe |
| [existing-tests.txt](existing-tests.txt) | Existing spectrogram-filtered UI suite: 48 passed, one ignored, GPU paths exercised on this machine |
| [existing-offline.txt](existing-offline.txt) | Existing synchronized offline frame differential |
| [scratch-initial.txt](scratch-initial.txt) | First CPU/allocation/gap/coverage/timestamp measurements |
| [scratch-final.txt](scratch-final.txt) | Repeat, plus gap reproduction through public sample ingestion |
| [probe.patch](probe.patch) | Exact final scratch fixture, applicable to the source baseline; not part of compiled production code |
| [render-tests.txt](render-tests.txt) | Post-snapshot renderer spectrogram suite: 9 passed |
| [offline-goldens.txt](offline-goldens.txt) | Post-snapshot spectral goldens: 5 passed unchanged |
| [active-work-overlap.txt](active-work-overlap.txt) | Final inspected implementation heads, cumulative file lists and worktree status |
| [Review reports and dispositions](review-disposition.md) | Independent initial verdicts, source validation, accepted/qualified/declined recommendations |

Existing commands, run before scratch results were used:

```sh
cargo test --release -p harmonigraph-ui profile_picture_panes -- --ignored --nocapture --test-threads=1
cargo test --release -p harmonigraph-ui spectrogram -- --test-threads=1
cargo test --release -p harmonigraph-offline what_the_heatmap_costs_a_frame -- --ignored --nocapture --test-threads=1
```

Reproduce the scratch fixture in an owner-managed worktree at the baseline, with a branch created before applying it:

```sh
git apply /path/to/docs/evidence/spectrogram-audit/probe.patch
cargo test --release -p harmonigraph-ui audit_scratch -- --ignored --nocapture --test-threads=1
git apply -R /path/to/docs/evidence/spectrogram-audit/probe.patch
```

The audit removed that patch from runtime/test source after measurement.
Do not leave it applied in an implementation branch or treat these ignored probes as committed regression coverage.
The first invocation was refused by the sandbox because sccache could not start;
the recorded successful runs used the app's approved command execution.
Build duration is not part of the printed CPU timings.
Raw transcripts and the unified patch retain original whitespace;
diff-whitespace validation applies to Markdown documentation, while the archived patch is checked by applying it against the unchanged source.

## CPU boundary and fixture reach

`audit_cpu_and_gap` calls the production `Plan::new`, `run_for`, and `frame_data`.
The timed/allocated region is `run_for` plus `frame_data`;
planning and history ingestion are outside it.
The test reads the existing `Counting` allocator's counters before and after that region.
An allocator request counts a `realloc` at its new requested size, so summing requests is not a peak-live memory measurement.
Two relaxed counters add probe overhead, and this machine was not globally isolated from every other application.
No CPU total is labeled GPU time or host frame time.

Synthetic history columns contain 3,828 bytes:
a nonzero floor at 65 and a moving 230-byte peak among 3,000 interior bins.
This deliberately makes MAX-tail updates nontrivial;
it is not silence and not a repeated identical column.
It does not reproduce every spectrally correlated audio workload, so CPU timings are stage costs for this fixture rather than universal host costs.
12/180/600-second windows are populated for window + 5 seconds at the live 8 ms rate before first drawing.
The 600-second case pushes 75,625 columns and reaches coarsened tiers;
the 12-second case reaches the first tier merge.
16 update frames warm the handoff, then 120 updates are measured, two columns per update (62.5 updates/s).
All report one initial refold and one full upload, so steady-state results are not accidentally timing repeated cold rebuilds.

CPU receipt is explicitly acknowledged by storing the returned serial into its shared atomic after each frame.
No GPU upload is performed in this scratch loop.
This models a successfully prepared callback and makes the production incremental delta path reachable;
without it every advanced run would correctly fall back to a full upload.
The existing GPU sequence tests separately verify that acknowledgements, skipped callbacks, negative/wrapping keys, and subsequent pixel reads agree.

Each tuple is `(CPU milliseconds, allocation requests, requested bytes, dirty slabs, visible slabs, GPU capacity slots)`.
The printed `warm_p50` tuple belongs to the median-time sample;
`mean_requested_bytes` and `mean_allocs` aggregate the measured window.
The 120-frame p95s are descriptive, not a statistical confidence interval.

| Window / depth pixels | Initial p50 / p95 ms | Repeat p50 / p95 ms | Mean requested MB/update | Visible slabs | Dirty slabs |
|---|---|---|---|---|---|
| 12 s / 1,024 | 0.226 / 0.840 | 0.239 / 1.046 | 2.881 | 751 | 2–3 |
| 180 s / 512 | 0.070 / 0.129 | 0.072 / 0.102 | 1.353 | about 353 | 1–2 |
| 180 s / 1,024 | 0.142 / 0.210 | 0.143 / 0.215 | 2.702 | 704–705 | 1–2 |
| 600 s / 1,024 | 0.124 / 0.214 | 0.119 / 0.159 | 2.252 | about 587 | 1–2 |

The 12-second p95 is noisy and much greater than the median in both runs.
Do not use these samples to promise a frame-rate improvement;
the whole timed stage bounds possible savings at this fixture, and a replacement has its own work.
A frozen-history frame and a pitch-only zoom/row change allocate zero within this boundary and return no dirty slabs.
Those single sub-microsecond samples establish the path, not a reliable nanosecond performance comparison.
The current data path needs optimization while data changes, not a new pitch-zoom cache.
The existing 0.121 ms whole-pane probe is not comparable to the 0.226 ms scratch subset:
it has only about five seconds of history and does not acknowledge callbacks, so `accept` skips the full `SentRun::moved` scan the scratch reaches.

## Defect reproductions and eliminated explanations

### SG1: gap expansion before retention

At 12 seconds / 1,024 depth pixels, seed one second of history, prepare/acknowledge, stop feeding samples, then resume after 1/60/600 seconds.
The direct-column arm provides an exact time grid.
The final arm instead feeds 60 batches of 800 actual samples through `AudioSpectrum::push_samples`, then two batches at shell times 601 s and 601 + 1/60 s.
It uses the production anchor snap, analyzer, quantization, history retention, plan and aggregation.
The input is constant nonzero amplitude;
signal content is irrelevant to zero-gap capacity, but real FFT columns must exist before preparation.

| Gap | Final visible slabs | Retained `grid.power.capacity()` | Requested bytes during resumed preparation | CPU ms (initial / repeat) |
|---|---|---|---|---|
| 1 s | 127 | 489,984 | 979,340 | 0.036 / 0.036 |
| 60 s | 1,032 | 15,679,488 | 34,900,360 | 2.433 / 2.286 |
| 600 s | 1,032 | 250,871,808 | 506,390,920 | 14.380 / 14.759 |
| about 600 s, actual sample ingestion | 1,032 | 250,871,808 | 506,390,920 | repeat only: 18.348 |

Source explanation:
`SlabGrid::fold` iterates every absent key and extends `power`;
`SpectrogramAgg::view` later drains prefixes but keeps the allocation.
Post-review source validation also identifies `rebuild`'s `partition_point(...).min(first)` as a second entry to the same large fill when layout/rung changes.
The archived measurements exercise warm increment;
no separate cold-gap timing is claimed.
Eliminated:
FFT time (outside the measured region), GPU allocation/upload (no GPU in this fixture), non-acknowledged callbacks (serial explicitly acknowledged), and an oversized final visible run (length is trimmed).
No proposed repair was tried or reverted;
only probes were added and removed.
Still needed:
bounded implementation, exact short-gap/held-copy equivalence, huge-gap intermediate bounds, and an actual host pause/resume check if a host-specific claim is made.
Stopped transport with continuously delivered silent audio does not trigger this gap;
the sample feed must cease or skip time.
Closing the editor alone continues ingestion in the background.
Engine suspension, plugin/track deactivation and a large forward anchor snap are candidate host reproductions, not measured host outcomes.
SG1 acceptance now names both fold drivers, pre-allocation bounds, post-gap far-edge extent, and the fewer-than-two-slabs no-callback path.

### SG2: offline batch endpoint mismatch

The probe mirrors the actual `render.rs` caller convention:
feed one frame of 48 kHz audio from a chunk beginning at `now`, but date its newest sample with `now`.
After one second, compare the history's last center with `(frames_seen - 1) / sample_rate - column_lag`.
Offsets are −33.312500, −16.645833, −8.312500 ms at 30/60/120 fps.
The one-sample difference from a whole frame follows the existing last-sample convention.
The same error appears at all three frame rates and scales with frame duration.

Eliminated:
shader/geometry, GPU timing, clock wall-time jitter, and the intentional half-window lag (subtracted on both sides).
The current `a_live_column_is_stamped_at_the_middle_of_its_window` test checks the callee with a correctly dated batch;
it does not exercise the faulty offline caller.
The scratch test mirrors that caller but is not a render integration test.
Still needed:
a fixture through actual slicing/render orchestration with nonzero audio origin, trimming, fractional rates and a partial tail;
define lookahead explicitly before fixing.

### SG3: sparse whole-song observations miss a burst

Two seconds of mono audio at 48 kHz contain a sine burst only from 0.320 to 0.330 seconds.
Call the real `WholeSong::precompute` with start/origin zero, default Balanced/one taper, and requested spans 12, 180, 600, 1,800 seconds.
Peak stored bytes are respectively 203, 203, 190, 0;
output column counts are 229, 67, 21, 8. The 1,800 s hop ends near 0.27466 and 0.54932 s;
their 0.17067 s windows leave the burst entirely between observations.
This is an allowed render window longer than its soundtrack;
only the two seconds of input are analyzed, so its printed CPU times must not be presented as 30-minute export times.
The same initial uncovered interval exists if the rest of the requested window is supplied as silence.

Eliminated:
downstream MAX, quantization alone (every FFT sample of the burst is absent from the sampled windows), pitch filtering, LUT mapping and GPU rendering.
The two-second deterministic whole-song tone test never reaches a hop wider than the window and cannot cover this defect.
The late-window pre-roll test reaches trimming but likewise never reaches the long-span hop.
Still needed:
sweep burst phase, FFT size and sample rate, compare to a regular-hop reference, choose a quality policy, and measure long-window CPU/memory cost before selecting preaggregation.
The independent reviews add one/five-taper comparisons, explicit raw-column memory admission, and non-nested placement-grid fixtures to that gate.
The 323 MB Fast/192 kHz/30-minute raw-column example is calculated from a one-window hop, not a measured allocation or export duration.

## GPU, composition, and measurement limitations

The existing offline probe uses nine rotated repetitions of each heatmap/curve/sliver variant, discards the first 16 frames, and takes paired differences before the median.
Its scatter is half the span after dropping the lowest and highest samples, not a standard error.
The largest result is 2.156 ±0.235 ms at 2.10 Mpx;
the complete heatmap frame median is 4.294 ms. Those include whole-frame synchronization/readback and shared work;
neither number identifies fragment time.
At 0.03/0.10/0.39/1.57 Mpx, heatmap differentials were 0.218/0.209/0.108/1.411 ms with scatter 0.152/0.102/0.048/0.522 ms. Retain the noisy/nonmonotonic rows rather than fitting an FPS curve.

The historical [#519 report](https://github.com/yan-h/harmonigraph/issues/519#issuecomment-5465876137) reports about 0.3 ms CPU and 0.1 ms GPU on a real Bitwig surface, but excludes tessellation and gives no exact pixel/span fixture.
Its CPU and GPU figures should not be added as a validated 0.4 ms critical-path cost:
host CPU and GPU overlap.
The same issue documents a failed GPU bracket whose trailing independent attachment began before preceding drawing finished.
Today's vendored egui timer still has that independent 1×1 tail topology (`vendor/egui-baseview/src/renderer/wgpu/renderer.rs:169,673`).
We did not reproduce that timer defect in the host or claim a corrected GPU duration here.
SG6 therefore requires a workload-scaling/dependency sanity check before treating its result as a trustworthy shader budget.

Existing GPU tests and offline probes ran on the actual Metal adapter;
GPU execution for correctness is not a GPU timing measurement.
No new timestamp scopes, raw-bandwidth measurement, RSS measurement, live multi-window host experiment, or cold-driver-cache reset was performed.
These are explicit remaining gates rather than grounds to extrapolate CPU allocation sizes into promised frame rates.

## Review packet

Review the complete plan and all SG1–SG6 briefs, this evidence, both scratch logs and the patch, and the actual source at the baseline.
The review is of technical proposals, priority and dependency claims, not just prose or diff style.
The frozen snapshot and independent reports/disposition are recorded in the review files beside this packet.
Both initial verdicts target commit `994fbac5657193d0e0b14706808edc27c9807a9c`.
`initial-snapshot.sha256` remains the immutable manifest of that version;
verify against `git show 994fbac5:<path>`, because this README and the plan now include accepted revisions.
The reports are preserved verbatim in `.txt` files, including recommendations qualified or declined by [the disposition record](review-disposition.md).

Post-snapshot baseline validation, with no golden changes:

```sh
cargo test --release -p harmonigraph-render spectrogram -- --test-threads=1
cargo test --release -p harmonigraph-offline frame_on_record -- --nocapture --test-threads=1
```
