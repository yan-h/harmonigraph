# Independent reviews and validated dispositions

The initial review target was documentation commit `994fbac5657193d0e0b14706808edc27c9807a9c`, against unchanged production source `70e9f48dddb3c14e5cb2d2173d5604f8a4c34e84`.
Both reviewers received the [same complete brief](review-brief.md), plan with all six component briefs, source access, evidence and probe patch.
No initial report was shared with the other reviewer.
The proposal packet stayed frozen until both returned.
The original [manifest](initial-snapshot.sha256) describes that commit, not the subsequently revised plan/README;
verify its paths with the contents from `git show 994fbac5:<path>`.

- [Claude initial report, verbatim](claude-initial.txt): requested `$claude-review model=opus effort=xhigh fix=false`, run through that skill's review script with the baseline and complete technical brief. Verdict: accept with corrections to SG1/SG3 and priority.
- [Astra initial report, verbatim](astra-initial.txt): independent read-only `gpt-6-astra` agent, fresh context containing the complete packet instructions. Verdict: support the direction and order, with three SG3 contract corrections.

Both reports are source reviews, not independent remeasurements.
Claude could not verify the other branches' write sets in its read-only tool environment;
the coordinator and Astra inspected the relevant Git diffs, and the coordinator refreshed active PR heads and worktree status after the reviews.
The revisions below were independently checked against source by the coordinator and applied to documentation/issues only.
There was no second independent verdict on the revised proposal.

## Finding-by-finding disposition

Paths below abbreviate `crates/harmonigraph-` as in the main plan and refer to the source baseline unless a branch is named.

| Finding | Disposition | Independent validation and resulting contract |
|---|---|---|
| Astra A1: a shared whole-song MAX grid cannot exactly serve arbitrary placements | Accepted | `ui/src/spectrogram.rs:639–663` uses `window / target_cols`; these partitions need not nest. A peak beside a new boundary loses its side after an earlier MAX. SG3 now requires per-distinct-grid accumulators or retained boundary information for exact streaming, budgets the admitted placements, and records the `offline/src/render.rs` layout-before-precompute overlap. Approximation/nested grids require visual acceptance. |
| Astra A2 and Claude F1: capped-hop raw columns violate the fixed output memory gate | Accepted, with a qualified alternative | `WholeSong::precompute` retains every sampled column. `1800 / (4096 / 192000) = 84375`, or 322,987,500 payload bytes before margins/metadata. SG3 distinguishes capped sampling with an explicit memory admission limit from bounded streaming. Streaming is required for a fixed output budget across accepted long windows. Decline silently increasing hop again at a column cap: that reintroduces the defect. The current approximately 6,554-column bound has margins/rounding, so it is not an exact hard count. |
| Astra A3: taper count missing from SG3 quality/cost gates | Accepted | `core/src/spectrum.rs` selects Hann for one taper and sine-family windows for multiple tapers, with one FFT per taper. Add compact one/five-taper phase sweeps and analysis-cost cases. Existing one-taper measurements do not establish a universal attenuation tolerance. |
| Claude F2: SG1 rebuild is also unbounded | Accepted | `SpectrogramAgg::rebuild` at `ui/src/spectrogram.rs:1079` uses `partition_point(...).min(first)`, admitting a retained pre-gap column, then invokes the same unbounded `fold` loop as append. The archived warm probe does not measure this second entry. SG1 now names both paths, assigns bounding before either fold, and requires cold/rung-change fixtures as well as warm append. |
| Claude F3: post-gap far-edge and fewer-than-two-slabs behavior | Accepted risk, qualified inevitability | `build:775` returns `None` below two slabs; pane drawing then omits the callback. Pane geometry derives its far edge from `layout.t_origin`. Resetting to post-gap samples would regrow a strip, but bounded allocation can instead preserve black slabs across the retained window. SG1 prefers that picture-preserving option and requires sequential resume images, first drawable callback and no stale image after reset. The report's claim that every bounded grid necessarily regrows is too strong. |
| Claude F4: put SG2 first and remove replay coordination | Priority accepted; scope/dependency qualified | The actual slice caller dates the newest sample with the slice's start, giving a recurring FPS-dependent error. SG2 now leads on correctness priority; SG1 can proceed independently while replay edits are coordinated. Whole-song precompute has its own clock, so “every frame of every export” overstates the affected heatmap scope. Endpoint/origin/tail/lookahead semantics and the actual caller integration fixture belong together; a one-line-looking fix does not remove the observed shared-file coordination requirement. There is no blanket dependency on completion of auto-tuning. |
| Claude F5: SG4 A needs a prepare-stage fixture | Accepted | `render/src/spectrogram.rs:503` allocates padding inside `CallbackTrait::prepare`, outside both existing CPU timing boundaries. SG4 A now explicitly calls for a warm renderer-stage allocation probe with zero/1–3 dirty slabs, aligned production and odd generic bin counts, separating our staging from driver allocations. Source proves the avoidable allocation; no timing benefit was measured. This remains an optional tiny early improvement, not an FPS claim. |
| Claude F6: derive dirty keys before adopting circular storage | Accepted as a measured intermediate step | Source shows new/tail mutations in `fold` and first/held repairs in `view:1205–1227`; `SentRun::moved:370` currently scans bytes. SG4 B now evaluates dirty tracking and snapshot reuse while keeping the flat store/full recovery packet; circular storage becomes C only if remaining copies/shifts matter. Retain new-key, dropped-ack, full-upload threshold, rung/rebuild and GPU eviction safeguards. The report's “no new storage” and exact savings are not established performance results; conservative dirty marking may increase upload volume. |
| Claude F7: reconcile the 0.121 ms pane and 0.226 ms subset | Accepted | Different history lengths and callback acknowledgement produce different branches. Without acknowledgement, `accept` bypasses `SentRun::moved`; the existing pane result cannot bound the scratch workload's savings. Added an explicit explanation to the plan/evidence. |
| Claude F8: shared fold needs an explicit budget owner | Accepted | `aggregate_slabs` and `SpectrogramAgg` both call `SlabGrid::fold`; whole-song does not use the live `keep` budget. SG1 bounds through its live owner or an explicit driver budget, with non-vacuous long-gap checks in both drivers. It must not apply the live cap to whole-song by accident. |
| Claude F9: clarify reach and candidate host actions | Accepted, hypotheses kept separate | `push_samples` can snap its anchor after a large offset; editor/background share continuous draining. The live gap can arise from missing feed or skipped time, but no Bitwig action was measured here. Engine suspension, track/plugin deactivation and anchor jumps are named candidate host reproductions, not established host defects. Closing the editor and silence alone are explicitly excluded as triggers. |
| Claude F10: SG6 GPU attribution must wait for lattice #642 A | Declined as a hard dependency | Lattice `GpuTimer` belongs to `render/src/lib.rs:1785`; `EguiGpuTimer` belongs to `vendor/egui-baseview/src/renderer/wgpu/renderer.rs:65` and has its own queries/tail. The spectrogram draws in egui's pass. The inspected #662 implementation changes the lattice timer, not the vendored egui timer. Similar independent-tail topology warrants shared methodology, but landing #642 does not validate the egui bracket. SG6 retains its own scaling/dependency gate and coordinates shared text/timing work. |

## Coordinator corrections and refreshed overlap

Two additional source corrections were made without changing the measured findings:
`ChannelBank::power_sum` divides by channel count and therefore returns mean power;
whole-song spectral composition hides the current curve/voice bars even though the frame loop still feeds live analysis.
Avoiding that feed for layouts with no consumer is a conditional SG6 experiment after consumer attribution, not a production change here.

The final active-head check inspected #653 `f548b13a`, #661 `943c8efb`, and #662 `b8ecafb6`, in addition to the original tuning branches.
Their worktrees were clean at that check.
The canonical branch still overlaps offline rendering/replay and shared UI/frame ownership but does not edit dedicated spectrogram CPU/GPU modules.
Trail scratch edits `scene/src/trail.rs`.
The lattice GPU batch edits `render/src/lib.rs`, shared `text.rs`, lattice/text WGSL and renderer tests;
its text pipeline/depth/bloom interface changes are an actual SG6 coordination boundary.
These are time-stamped observations, not permanent disjointness guarantees or validation of those PRs' claimed performance.

## Revised order and validation

SG2 leads on recurring correctness, with its caller/tests coordinated with replay;
SG1 is immediately independent.
SG3 resolves coverage, attenuation, memory and placement scope together.
SG4 A is a tiny independently measurable option, B tests cheaper flat-store improvements, and C needs evidence of remaining cost.
SG5 follows measured retention/reopen/fidelity tradeoffs.
SG6 provides attribution where needed and admits GPU/text/export rewrites only after their bottlenecks are demonstrated.

After the initial packet, the coordinator ran the existing renderer spectrogram suite (9 passed) and offline spectral goldens (5 passed, unchanged, no blessing).
The earlier UI spectrogram suite passed 48 tests with one ignored, and both existing performance probes plus the temporary scratch probes completed.
These checks validate the baseline and evidence apparatus;
they cannot validate unimplemented repairs.
Final documentation checks cover Rust formatting, semantic Markdown layout, Markdown diff whitespace, local links, initial-commit checksums, clean applicability of the archived probe, and absence of runtime changes.
Raw tool transcripts and the unified probe patch retain their original whitespace, including blank context lines;
they are excluded from the Markdown whitespace check.
The source and probe patch are unchanged by the review revisions.
No release build is owed for this documentation-only change, and no shared DAW slot was touched.
