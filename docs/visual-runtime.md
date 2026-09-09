# Visual runtime ownership

Issue [#732](https://github.com/yan-h/harmonigraph/issues/732) separates synchronous progression from editor layout and per-surface graphics.
`SharedState` is the editor aggregate;
it owns a `PictureState` and a `Workspace`.
The offline renderer constructs only `PictureState`,
and replay feeds `VisualRuntime` directly.

`VisualRuntime` owns the tracker, analyzed spectrum and bounded history, configuration observation, and explicit time progression.
It borrows the existing `AppearanceDocument` when observing parameters and advancing.
`LiveInput` retains the plugin's sole publication consumer, audio consumer, clock mapper, and reusable scratch for the plugin lifetime.
It drains into runtime storage without access to a dock or recording controls.
Audio producers retain their existing independent, nonblocking contract;
this change introduces no queue, worker, snapshot, or alternate ingress for #742.

`Workspace` owns the dock, folds, interface preferences, interaction, and shell actions.
The dock traverses its own tree while `Viewer` borrows the picture and interaction separately.
Picture panes cannot replace the dock or access recording controls.
Appearance-only settings panes borrow the appearance document,
and the console borrows the runtime.
The reset-layout request is consumed after traversal;
there is no temporary empty dock or write-back that can overwrite the request.

`SurfaceState` owns viewport reports, divider geometry, glow row histories, and spectrogram aggregation/GPU mirrors.
Spectrogram preparation borrows bounded history separately from surface storage.
The audio ring's levels and gate envelope remain shared between dock and preview;
glow rows, slab geometry and GPU ownership remain distinct per surface.
Existing repeated/discarded-pass behavior and callback ownership remain unchanged.

## Advancement and remaining host boundaries

`VisualRuntime::advance` observes parameters before pruning,
so an increased Fade is honored in the same frame.
`advance_time` performs the same aging without parameter observation or drawing.
The closed-window scheduler drains and advances with the last observed parameter mirrors,
including the existing default mirrors before first open.
Closed-only persisted-document adoption still happens before draining,
so a restored analysis window affects the first consumed audio.
An open window's unsaved controls are never replaced by its last saved blob.

Recording synchronization remains once per GUI callback,
with the existing twenty-callback stopped-transport debounce.
It is not driven by closed runtime ticks.
The open check, `try_lock`, window/context construction and restore boundaries remain in the host shell.
The shared mutex still covers an open UI build;
this extraction does not shorten that lock or change its scheduling.
Teardown still joins the existing background drainer and consumes its final owned publication payloads.

Raw spectral history now expires after the existing 610-second retention window even when input has stopped.
Previously that trim waited for another sample batch.
The visible window is at most 600 seconds and whole-song rendering uses separate storage.
This does not promise that every cache allocation releases its capacity;
expiry can invalidate an existing aggregate key when its retained prefix changes.
Saved appearance/editor documents and their versions are unchanged.

## Fold sharing and keys

`AudioSpectrum` keeps one lazy Fold measurement for matching draws at the same logical frame time.
Its key is that time, the actual smoothed-spectrum revision, and normalized Fold width.
It recomputes in the next frame even if input is unchanged;
there is no cross-frame measurement reuse.
Audio changes at the same time increment the revision;
replacing the analyzer drops the measurement with its source identity.
Availability is checked before looking up the measurement,
so stale audio cannot keep a cached ring alive.
Camera, palette, viewport dimensions, node count, and other appearance controls are absent from the key because they do not feed the Fold calculation.
The shared ring envelopes and per-surface geometry are applied after measuring.

## Measurement method

Baseline: `a03290a2ac6b20929556467a30fe99b8dbccd742`, including the appearance-document merge.
The existing ignored headless profiler was extended before changing production hot paths:

```sh
cargo test --release -p harmonigraph-ui profile_visual_runtime -- --ignored --nocapture --test-threads=1
```

Each case warms 60 frames and measures 600 frames at 1600×1000 points and 2× device scale.
Active cases feed a six-tone chord at 48 kHz, 800 samples per frame, plus six held MIDI notes.
Quiet cases have neither audio nor held notes.
The simultaneous case selects the real Video tab and asserts that both lattice surfaces draw;
Fold reading and a visible audio ring are explicit.
Total time includes analysis, UI construction, and egui tessellation;
source-fixture generation is outside the timer.
The report includes minimum, median, p95, p99, maximum, and allocation counts.
No-drawing cases perform the same feed and runtime progression without building a UI.

The harness excludes GPU callback preparation/submission and native host/window contention.
It does not measure native idle CPU, input-to-display latency, window-open duration, or mutex-hold tails in Bitwig.
Existing lifecycle tests establish ordering and data preservation,
not host latency.
Source inspection still shows `egui-baseview` constructing `run_ui` before its presentation/repaint gate.
No timer, polling, presentation, or event scheduling change is included.

## Results (2026-09-09)

The baseline test executable was retained before production edits,
then compared with the final release executable using the same fixtures and compiler/profile/dependency features.
All task compilation had finished before sampling.
Six runs per executable alternated order;
the table reports the median of each run's statistic,
not a pooled percentile.
Times are milliseconds,
and allocations are the median count per frame.

| Workload | Total p50, baseline → final | Total p95 | Total p99 | Allocations |
| --- | --- | --- | --- | --- |
| Quiet dock | 0.147 → 0.146 | 0.223 → 0.234 | 0.307 → 0.303 | 661 → 647 |
| Active dock | 0.578 → 0.575 | 0.771 → 0.777 | 0.868 → 0.876 | 796 → 782 |
| Active dock + preview | 0.788 → 0.724 | 1.028 → 0.954 | 1.129 → 1.072 | 1023 → 1006 |
| Active without drawing | 0.161 → 0.164 | 0.239 → 0.249 | 0.278 → 0.329 | 2 → 2 |
| Quiet without drawing | <0.001 → <0.001 | <0.001 → <0.001 | <0.001 → <0.001 | 0 → 0 |

Dock plus preview reduced median total CPU work by about 8.1% in this harness.
Its UI-and-tessellation portion decreased from 0.612 to 0.551 ms,
while analysis remained around 0.163–0.165 ms.
The single dock's median is essentially unchanged.
Removing the temporary empty `DockState` eliminates allocations in every dock frame;
sharing Fold removes additional work when both surfaces consume it.

The initial no-drawing tail increase is retained in the table rather than labeled neutral.
Baseline per-run p99 ranged from 0.264 to 0.287 ms;
final ranged from 0.280 to 0.844 ms,
with a largest observed final outlier of 8.769 ms.
Those outliers were almost entirely inside the separately timed analysis feed,
while advancement's p99 was at most 0.002 ms.
No timing sample identifies an operating-system cause by itself.

A bounded follow-up ran three more pairs in fixed final-then-baseline order,
without compilation or fixture changes:

| Reverse-order pair | No-draw p50, baseline → final | No-draw p99 | Analysis p99, baseline → final |
| --- | --- | --- | --- |
| 1 | 0.163 → 0.162 | 0.281 → 0.283 | 0.281 → 0.282 |
| 2 | 0.160 → 0.161 | 0.269 → 0.264 | 0.268 → 0.264 |
| 3 | 0.162 → 0.160 | 0.278 → 0.280 | 0.278 → 0.279 |

The larger tail increase did not reproduce in that reverse-order check.
This supports an order-sensitive measurement qualification,
not a blanket no-regression claim or a native latency conclusion.
The comparison does establish reduced duplicated Fold work,
lower dock allocations,
and approximately unchanged ordinary single-dock/runtime medians.

## Validation

The workspace suite passed 1,544 tests with no golden rebaseline.
Focused additions exercise no-draw history aging and analysis uniqueness,
actual dock/Video Fold sharing across discarded layout passes,
same-time audio and width changes,
parameter observation before pruning an increased Fade,
and recording debounce at nineteen/twenty GUI callbacks after one hundred no-draw ticks.
Existing restore-before-first-open, open/closed handover, simultaneous surfaces, real dock gestures/reset-layout,
and #739 CPU/GPU row-owner regressions remain in their original suites.

Independent read-only reviews covered UI/surface ownership, cache dependencies and fixture reach,
then plugin lifecycle, configuration/replay ordering, recording cadence and silent-history expiry.
The final reviewed source had no outstanding substantive finding.
Workspace clippy with warnings denied and the strict rustdoc/local-Markdown gates also pass.
GitHub Actions remains the full integration gate for the draft PR.
