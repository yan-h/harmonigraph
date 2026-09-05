# Spectrogram rendering audit and implementation plan

## Status, evidence, and scope

Audited on 2026-09-05 from current `origin/main`, `70e9f48dddb3c14e5cb2d2173d5604f8a4c34e84`, in the Codex-managed `codex/spectrogram-rendering-audit` worktree.
This is an audit and documentation change;
none of the proposed rendering changes is implemented.
Tracker:
[#654](https://github.com/yan-h/harmonigraph/issues/654).
The method follows [the lattice plan](lattice-rendering-plan.md) and [#643](https://github.com/yan-h/harmonigraph/issues/643), but the dependencies are established independently for this path.

**Recommendation:** prioritize offline timestamp correctness (SG2), coordinate its replay-file edits, and start bounded gap recovery (SG1) independently while that coordination happens.
Resolve whole-song coverage and memory together (SG3), take the tiny upload-allocation improvement, then measure a staged CPU handoff refactor.
Keep the current GPU resampling architecture unless validated GPU measurements justify replacing it.
The strongest performance evidence concerns CPU allocation and resource retention;
the evidence does not support a combined FPS forecast.

| Component | Issue | Reviewed priority |
|---|---|---|
| SG1: bounded gap recovery | [#655](https://github.com/yan-h/harmonigraph/issues/655) | Immediate independent correctness/resource repair |
| SG2: offline slice timestamps | [#656](https://github.com/yan-h/harmonigraph/issues/656) | Highest recurring correctness priority; coordinate replay edits |
| SG3: whole-song temporal coverage | [#657](https://github.com/yan-h/harmonigraph/issues/657) | Early correctness/quality decision |
| SG4: CPU handoff and staging | [#658](https://github.com/yan-h/harmonigraph/issues/658) | Tiny staging slice early; larger work measured |
| SG5: lifetime/retention | [#659](https://github.com/yan-h/harmonigraph/issues/659) | After explicit fidelity and memory gates |
| SG6: attribution and experiments | [#660](https://github.com/yan-h/harmonigraph/issues/660) | Measurement as needed; rewrites conditional |

[Evidence and reproduction packet](evidence/spectrogram-audit/README.md) contains commands, fixture reach, raw results, limitations, and the temporary probe patch.
Labels used here are **S** (established by source), **M** (measured in this audit), **H** (historical measurement), **E** (calculated estimate), and **Q** (hypothesis requiring measurement).
These are distinct kinds of evidence, not confidence scores.

## Actual architecture

| Stage / owner | Work, retained state, and boundary |
|---|---|
| Audio thread, plugin `process` | Selects main/sidechain analysis audio, writes interleaved frames to an `rtrb` ring; records the selected audio separately. The 131,072-sample ring drops on overflow and never waits for a renderer. Channel count and sample rate are published separately. No spectrogram GPU work runs here. |
| `EditorShared`, plugin lifetime | Owns `SharedState`, `AudioSpectrum`, note/audio consumers, reusable drain vectors, and a clock since plugin instantiation. An open editor holds its mutex through the frame, drains audio, and runs analysis before UI construction. |
| Background analyzer | While the editor is closed, polls every 20 ms, checks `is_open`, takes `try_lock`, restores saved settings, and calls the same drain path. It retains history while no pane is visible. A reopen can wait behind one in-flight drain; the ring, not the intended poll interval, bounds that drain. Drop joins the worker. |
| `AudioSpectrum::push_samples` | Uses `ChannelBank` and an 8 ms sample-count hop, not the UI frame clock. Per-channel powers are averaged; raw spectra become byte columns. The separate attack/release display filter serves the current curve, not history. Anchor smoothing maps sample counts to shell time; measurements are stamped half a window behind the newest sample. |
| Core `SpectrumHistory` | Owns boxed arrays of 3,828 dB bytes in seven age tiers. Fine tier 2,048 columns, six coarse tiers of 1,024; MAX-merge pairs and midpoint their time. UI retention trims at 610 seconds, independently of the displayed 1–600 second live history span. Whole-song's drawn-window floor is separately 0.05 s. |
| Per-surface CPU state | `SpectrogramSurfaces` indexes dock 0, video preview 1, and offline placement IDs. Each surface owns `SpectrogramAgg`, `GpuGrid`, LUT metadata, and slab-width hysteresis. The analyzer and history are shared. |
| Frame planning and aggregation | `draw_spectrogram` selects columns on the shared roll time axis. `Plan` selects physical pitch rows and time slabs. Live slabs lie on a dyadic 16 ms-and-up ladder, capped at 1,024 target columns, with a 5% downward hold. Ring capacity is the next power of two of target columns plus eight. Whole-song uses up to 4,096 target slabs and no live ladder. |
| `SpectrogramAgg` | Retains a flat `SlabGrid` of MAX-folded byte slabs, centers, and held-gap flags. New columns update only the tail, but serving a view drains old vector prefixes, copies the visible run, and repairs its partial first slab and any held copies. Interior slabs preserve the finer data as originally observed, even after history tiers coarsen. |
| CPU → renderer handoff | `GpuGrid::accept` compares the fresh copied run with the previous `Arc<Vec<u8>>`, names changed slab keys, and carries a full run for recovery. A per-surface atomic serial acknowledges that `prepare` queued the writes. This is upload submission knowledge, not a GPU fence or a readback. Dropped callbacks cannot silently become the base of a later delta. |
| `SpectrogramCallback::prepare` | In egui-wgpu's prepare phase, lazily creates a format-specific pipeline and per-pane resources, queues grid/LUT/vertex/uniform writes, then acknowledges. It returns no extra command buffers and performs no separate spectrogram render pass. |
| `SpectrogramCallback::paint` | One triangle-list draw, normally 6–12 vertices, inside egui's color pass. Uses the full surface viewport and egui's scissor. No depth attachment, scene texture, bloom chain, or spectrogram readback. |
| Live composition | Spectral pane paints its bed/grid, heatmap, spectrum curve in its own region, roll/ribbons, now marks, and labels on top. Text and shadows use shared text/roll infrastructure; their preparation cost is not the heatmap shader's cost. |
| Offline composition | `render.rs` creates one egui context and wgpu renderer, replays events, feeds audio, calls the same `begin_frame`/`draw_pane` with distinct placement IDs, tessellates, prepares callbacks, and renders. `frames.rs` submits, copies the entire output, maps/waits, strips padded rows into a fresh RGBA vector, and emits it. This synchronization belongs to the complete export. |
| Whole-song offline | `WholeSong::precompute` analyzes the requested window plus input margins once, with a span-dependent hop; columns and window remain fixed for that render. Each surface folds once, then redraws a static grid with a moving playhead. The frame loop also continues live analysis, even though whole-song spectral panes hide the current curve and voice bars; other pane consumers must be checked before skipping that feed. |

Source anchors at the audit SHA:
`core/src/spectrogram.rs:92,147,168,225`;
`ui/src/spectrum.rs:16,75,108,204,407`;
`ui/src/spectrogram.rs:279,591,639,744,919,1091,1165`;
`ui/src/panes/spectral/spectrogram.rs:184`;
`render/src/spectrogram.rs:232,337,432,629`;
`render/src/shaders/spectrogram.wgsl:95`;
`plugin/src/editor.rs:334,425`;
`plugin/src/background.rs:265`;
`offline/src/render.rs:110,158,215`;
`offline/src/frames.rs:109`.
All crate paths in this document are relative to `crates/harmonigraph-` where abbreviated.
Use the named symbols as the stable navigation points after implementation.

### What the pixels mean

**S:** history stores absolute dB in half-dB steps from −120 to +7.5 dB.
Time aggregation is bytewise MAX, preserving a peak that was actually analyzed within each slab.
The GPU storage buffer packs four bytes per `u32`;
slabs are padded to four-byte alignment, which 3,828 already satisfies.
The heatmap is not uploaded as an RGBA picture.

The shader maps each bucket through the level window and pitch tilt, **clamps each bucket to [0,1] before resampling**, and then area-weights levels across the pitch footprint.
For magnification it interpolates neighboring bucket centers.
It blends two neighboring time slabs in level space, then indexes a 4,096-entry `Rgba8Unorm` gamma-space LUT using `textureLoad`.
There is no sampler, hardware bilinear filtering, mip chain, or order-4 power mean in this current heatmap path.
The pipeline uses the gamma or linear fragment entry point according to target format, matching egui's encoding convention.
Intensity averaging preserves a level-space interpretation, not physical acoustic power or integrated RGB luminance through a nonlinear gradient.

The newest slab's center is held through the leading sliver, with geometry split at the bend;
interpolating one quad across that bend would rescale the data once per slab.
Startup clips the far edge to available history instead of stretching a few columns over the entire window.
Stale input stops the near edge after 120 ms plus half the analysis window.
One missing slab holds its predecessor;
larger gaps are black.
Preserve those distinct cases.

FFT windows (4,096/8,192/16,384 samples) constrain frequency and time resolution upstream.
The 8 ms hop samples overlapping windows more often;
it does not create new frequency resolution.
At long live spans, the slab cap rather than the FFT determines displayed time detail.
Changing the cap also changes the core history resolution guarantee, so it is not a renderer-local constant adjustment.

## Retained values and cache audit

| Value / current key | Missing-input check | Unnecessary-input check and conclusion |
|---|---|---|
| `RunKey`: first flat column index, total stored count, newest time bits, bucket bits, whole/live | Whole-song start/span/content identity are absent. Safe for the current immutable per-render `WholeSong`; unsafe if future preview code replaces a set or changes trim without resetting surfaces. Do not claim a currently reproduced stale-window bug. | Pitch range, rows, level, tilt, palette, frame time, and general `SpectrumConfig` correctly stay out. Count/newest identify possible history changes even when the resulting MAX is unchanged. Replacing these with dirty-range metadata needs an explicit history mutation contract, not blind key deletion. |
| Aggregator: bucket bits, last folded time, target within retained grid | Depends on ordered history, gap semantics, retained fine slabs, and first-slab repair. A backward move/reach miss refolds. The gap allocation defect below violates bounded work before trimming, not the GPU acknowledgement. | Rows/colors correctly absent. Coarse-tier rewrites deliberately do not invalidate interior slabs. A generation for every tier merge would restore the old refold-per-frame failure. |
| `SentRun` / delta: capacity, acknowledged serial, slab key range and byte equality | New keys must be dirty even if their bytes equal an old lap. Missing prepare means full recovery before the next advanced run. Full immutable run remains available if renderer resources are evicted independently. | Scans/copies all visible bytes on changing runs despite usually 1–3 dirty slabs. This is established CPU work; its replacement must retain the correctness that the comparison currently supplies. |
| GPU buffer: generation/capacity/bin count, per pane | Missing/evicted buffer uploads whole even if the CPU believes its serial was acknowledged. Format change recreates the resource map. Bind group rebuild follows actual buffer/LUT allocation. | Generation change rewrites but already reuses a same-shaped allocation. Resize can leave an oversized valid buffer until the next accepted run; that is retention, not stale pixels. |
| CPU/GPU LUT: canonicalized sanitized gradient, then generation/length | Gradient normalization folds hue in achromatic/black/white cases without changing saved bars. Per-surface generation remains paired with its pane; a new pane always uploads. | Level/tilt are uniforms and correctly excluded. Equal gradients across surfaces may duplicate 16 KiB tables; little reason for a new global cache. |
| GPU pane map: stable ID and last egui pass | Eviction runs only from another spectrogram prepare. With the last spectrogram hidden, no sweep executes. Reappearance recreates/uploads safely; context drop destroys GPU objects and releases CPU upload mirrors. | 120 passes is roughly two seconds only while passes advance near 60 Hz; not wall time. CPU aggregators/sent runs have no analogous hidden-surface retirement. Map cardinality follows surface identities, not a demonstrated unbounded ID leak. |

`release_context_resources` forgets CPU statements about GPU contents while retaining CPU aggregation.
Do not equate reconstructability from `SpectrumHistory` with byte-exact recovery of a long-running aggregator:
old tiers have already discarded temporal detail.
A lifecycle refactor that discards those fine slabs can change the picture on reappearance even when all cache keys are internally consistent.

## Measurements and size estimates

Release-mode probes ran on an Apple M1 Pro, 8 CPU cores / 16 GB, Rust 1.92.0, with one test thread and worktree-local build output.
No Bitwig UI actions or new in-host timing were performed.
Raw logs and exact fixture definitions are in the evidence packet.

| Evidence | Result | What it does not measure |
|---|---|---|
| M: existing `profile_picture_panes`, 1000×900 points at ppp 2, six voices, one second warmup then four seconds | Spectral UI median 0.121 ms, minimum 0.115 ms | No tessellation, callback preparation, GPU execution, or audio analysis in the timed region. Only about five seconds of history; no tier merge. Callbacks are not acknowledged, so this is not a valid production delta-upload benchmark. |
| M: scratch, 12 s / 1,024 depth pixels, populated 17 s before first draw | Warm data-path p50 0.226 ms; 2.88 MB requested allocations/update, 5 allocations, 2–3 dirty slabs | CPU `run_for` plus `frame_data`; excludes analyzer, UI, driver, GPU and submission. Receipt is explicitly simulated. |
| M: scratch, 180 s / 512 depth pixels | Warm p50 0.070 ms; 1.35 MB/update, 5 allocations, 1–2 dirty slabs | Same boundary; not a complete frame |
| M: scratch, 180 s / 1,024 depth pixels | Warm p50 0.142 ms; 2.70 MB/update, 5 allocations, 1–2 dirty slabs | Same boundary; not a complete frame |
| M: scratch, 600 s / 1,024 depth pixels | Warm p50 0.124 ms; 2.25 MB/update, 5 allocations, 1–2 dirty slabs | Same boundary; old tiers are reached by 75,625 input columns before drawing |
| M: first data preparation in those cases | 1.53–3.32 ms, including first CPU LUT/fold setup | One cold data state each, already within a running process; not cold device/driver or editor startup |
| M: frozen history and pitch zoom/row change | Zero requested allocations, no dirty slabs; sub-microsecond data-path samples | A single key-hit probe, not a stable timing estimate; other UI/GPU work still occurs |
| M: existing offline pane probe, 2048×1024 | Paired heatmap minus sliver 2.156 ms, scatter 0.235 ms | Whole serialized render/readback pipeline. It is neither shader time nor an additive host-frame cost. |
| H: #519, Bitwig 2026-08-29 | Reported about 0.3 ms CPU, excluding tessellation, and about 0.1 ms egui GPU difference | One-decimal overlay, undocumented exact workload. Current timer uses the same independent tail-pass topology that #519 itself warns may under-bracket Metal work. Treat GPU attribution as provisional pending a scaling/dependency validation. |

The warm scratch row is 16 update frames of warmup plus 120 measured updates, two new 8 ms columns per update.
Each fixture reports exactly one initial refold and full upload;
subsequent frames reach the incremental and acknowledged-delta paths.
The data path still copies megabytes even when the GPU writes kilobytes.
The 0.121 ms pane result and 0.226 ms scratch subset are not comparable workloads:
the former has less history and, without acknowledgement, takes the full-upload branch that skips `SentRun::moved`'s byte scan.
Allocator-requested bytes include reallocations at their requested new size;
they are not peak live bytes, process RSS, or physical memory traffic.
Repeat timings, p95s, and raw results are preserved rather than selecting only the fastest run.

**E:** live GPU ring at the normal maximum capacity is `1032 × 3828 = 3,950,496` bytes (3.77 MiB);
4,096 whole-song slabs occupy about 14.95 MiB (boundary rounding may add a slab).
Full history payload is at most `8192 × 3828 = 31,358,976` bytes (29.91 MiB), excluding deque and box metadata;
age trimming can bind first.
A usual update sends 1–3 slabs, 3,828–11,484 payload bytes, plus an 80-byte uniform and 96–192 vertex bytes.
At 60 updates/s that slab range is about 0.23–0.69 MB/s per surface, not measured bus bandwidth.
Full upload allocates/zeros/scatters a capacity-sized CPU staging vector and queues that entire buffer.
The warm renderer branch also allocates a 3,828-byte padding vector even with zero dirty slabs.
The LUT is 16 KiB CPU plus GPU per surface;
the initial vertex buffer is 1 KiB.
CPU aggregation and the last sent run each retain another slab-sized array, with spare vector capacity in addition.

**M:** 60 s and 600 s input gaps at a 12 s span retain grid backing capacities of 15,679,488 and 250,871,808 bytes after trimming to 1,032 slabs.
The 600 s call requested 506,390,920 allocator bytes and took 14.38 ms in the initial probe.
The repeat took 14.76 ms;
feeding through the real `push_samples` API reproduced the same capacity and requested bytes, taking 18.35 ms for the subsequent data preparation.
This is about 239.25 MiB of retained vector capacity, not a 483 MiB live allocation or a GPU buffer of that size.
The normal 1,032-slot assumption also breathes to 1,034 slots for the returned gap fixture, because `ring_capacity` adds two to the visible retained run.
That is bounded output but does not bound the intermediate fill.

## Prioritized component briefs

The SG identifiers below name independently scoped issues, not proposed new runtime abstractions.
Effort is relative:
small is a narrow change;
medium changes an ownership or time contract;
large adds an alternate pipeline.
Every performance change has a measurement gate;
correctness repairs need not justify themselves with FPS.

### SG1 — bound gap expansion and retained capacity

**Problem/evidence:** `SlabGrid::fold` materializes every missing slab before `SpectrogramAgg::view` trims it with `Vec::drain`, which does not shrink capacity.
Both live entry points are unbounded:
`window`'s incremental append and `rebuild`'s `partition_point(...).min(first)`, which can pull the pre-gap column back before the retention cutoff.
The measured warm fixture reaches append;
the cold/rung-change rebuild is established by source, not separately timed here.
M:
the 600 s gap above retains 239.25 MiB for a 12 s view.
Reach requires a ceased sample feed or forward timestamp skip, followed by samples while old history remains inside the 610 s retention.
Ordinary stopped transport that keeps delivering silence is not this case.
Closing the editor alone is not a trigger because background ingestion continues.
Q:
audio-engine suspension, plugin/track deactivation, or a forward anchor snap greater than one second are candidate host reproductions, not verified host actions in this audit.
The evidence packet includes both direct history injection and an `AudioSpectrum::push_samples` reproduction;
this is not claimed as a reproduced Bitwig stall.

**Smallest useful change:** let the live aggregator choose a bounded retained key interval before either appending or rebuilding, discard unreachable prefix work, and release already pathological excess capacity at a defined reset/rebuild boundary.
Avoid creating a huge vector and then fixing its length.
The bound belongs to the live owner, or an explicit budget supplied to the shared fold;
`aggregate_slabs` also uses `SlabGrid::fold` for whole-song and reference tests and must not silently inherit the live cap.
**Larger alternative:** a key-addressed circular CPU slab store shared with SG4's handoff design;
do not require that refactor to fix the defect.

**Effort/maintenance:** small–medium, localized to `ui/src/spectrogram.rs`;
a capped sparse-gap path adds an invariant but should remove the mismatch between retention length and allocation.
**Benefit/confidence:** high confidence in bounded allocation and recovery work;
steady-state FPS benefit is not the motivation.
**Visual risk:** skipping the wrong first slab, changing one-slab holds, extending stale energy into a real gap, losing the partial-edge repair, or re-reading coarse history.
Prefer preserving the current black-filled visible window across a long gap using only bounded slabs.
A reset to post-gap samples alone would instead regrow the far edge like startup;
that is an alternative picture change requiring explicit acceptance, not an inevitable effect of bounded storage.
`build` returns no callback for fewer than two slabs, so verify the first resumed frames and ensure no stale heatmap survives a reset.

**Dependencies and gates:** independent of tuning and lattice production interfaces;
sequence with SG4 edits to the same module.
Test 1, 2, 60, 600, and beyond-retention gaps, both cold rebuild and warm increment, at 12/180/600 s spans.
For huge gaps, assert intermediate and retained capacity stays proportional to admitted slots, rather than merely asserting final length.
Require ordinary/short-gap output equivalence, absolute timestamps, black real gaps, held-copy pruning, and successful GPU full/delta recovery after the gap.
Use sequential images across resume, including one post-gap slab, a rung-changing rebuild, far-edge extent, and the first drawable callback.
Exercise long gaps in both live and batch/reference drivers, with separate explicit budgets where applicable.
The existing held-trim tests must actually exceed retention;
preserve their fixture reach.

### SG2 — align offline streaming audio with the draw clock

**Problem/evidence:** `offline/src/render.rs` takes audio `[now, now + step)` and calls `push_samples(..., now)`, whose contract dates the newest sample of that batch.
M:
after one second at 48 kHz, 30/60/120 fps shift the recorded column centers by −33.3125/−16.6458/−8.3125 ms against sample indices.
The offset is almost one frame, not the already intentional half-window FFT lag.
It affects scrolling history;
whole-song precompute stamps on its own sample grid.

**Smallest useful change:** pass the actual final-sample timestamp of the supplied slice, or feed slices ending at the frame clock, consistently with the chosen lookahead convention.
Use clamped sample indices, audio origin, and partial final batches;
blindly adding one frame to `now` is incomplete.
**Larger alternative:** a small explicit audio-slice/time descriptor shared by offline feeding paths;
do not redesign the live analyzer's clock mapper for an offline caller error.

**Effort/maintenance:** small–medium;
centralizing endpoint semantics is simpler than implicit offsets.
**Benefit/confidence:** high-confidence correctness and frame-rate agreement;
essentially no expected throughput gain.
**Visual risk:** deliberate movement of heatmap ridges relative to MIDI and the playhead;
specify pre-roll/first-frame behavior and avoid double-correcting window lag.

**Dependencies and gates:** `offline/src/render.rs` overlaps active #617 recovery/configuration work;
implement against integrated source-aware replay or an explicitly coordinated disjoint slice.
No semantic dependence on #621's tuning solver.
An integration fixture must exercise `render.rs`'s actual sliced-audio caller at 30/60/120 and fractional fps, nonzero audio origin, late start, and a partial tail.
Compare timestamps and a known transient against MIDI, within one sample plus the defined analysis-hop quantization, not identical whole frames across FPS.
Preserve repeated-render determinism and explicitly assess expected golden changes.

### SG3 — prevent temporal blind spots in whole-song precomputation

**Problem/evidence:** the whole-song hop grows as `span / (4096 × 1.6)`, while the FFT retains only its window of samples.
At 1,800 seconds and 48 kHz Balanced, the hop is about 274.66 ms versus a 170.67 ms window;
entire intervals are never analyzed.
M:
a 10 ms 1 kHz burst at 0.320–0.330 s yields peak byte 203 at a 12 s requested span, 190 at 600 s, and zero at 1,800 s.
The fixture feeds two seconds of real audio into those requested windows;
it tests the actual precompute path, not synthetic MAX columns.
Extending the audio with silence cannot recover a burst no retained FFT window saw.
The formula crosses one window at approximately 559/1,118/2,237 seconds for Fast/Balanced/Precise at 48 kHz, and earlier at higher sample rates.
Even below that crossing, phase-dependent transient height already changes with a sparse hop.

**Policy decision first:** bound the sampling hop to an explicitly accepted coverage/attenuation policy, together with an explicit memory admission rule or streaming representation.
A hop no wider than the FFT window closes entirely unseen intervals but is not sufficient for phase-independent transient peaks because window endpoints are attenuated.
The current span-dependent hop limits raw columns to approximately 6,554 over the requested span, plus input margins and rounding.
Capping it while retaining `WholeSong.columns` instead grows column count roughly as duration divided by the accepted hop.
E:
at 30 minutes, Fast/192 kHz, even one window per hop requires about 84,375 columns × 3,828 bytes = 322,987,500 payload bytes before margins/metadata;
more overlap costs more.

**Smallest conditional repair:** keep raw columns only within a declared duration/rate/configuration-dependent memory admission limit, calculated before allocating.
Requests outside that limit must report the limitation or use the bounded alternative;
silently restoring a sparse hop would restore the defect.
This option does not meet a fixed output-slab memory budget and may be unacceptable as an export product constraint.
**Bounded architectural alternative:** analyze at the normal hop and stream MAX into bounded output grids without retaining every raw column.
This is required if the accepted policy must support long windows within a fixed slab budget.
It preserves the separation between observation rate and display resolution at increased analysis time and should reuse the existing MAX semantics.

There is no single exact shared grid for today's arbitrary placement widths:
whole-song uses `bucket = window / target_cols`, so grids need not nest.
MAX-folding once into 4,096 slabs loses which side of another placement's boundary produced a peak.
Exact streaming needs an accumulator per distinct output grid, or sufficient retained boundary information;
its total budget must therefore account for the admitted placement count and resolutions.
Resolve placements before precompute if their grids determine accumulation.
A shared-grid approximation or a change to nested placement grids is a separate visual policy with explicit acceptance, not an equivalent storage refactor.

**Effort/maintenance:** medium for capped sampling plus memory admission and its user-visible limitation;
medium–large for exact streaming preaggregation, placement planning and its `WholeSong` representation.
**Benefit/confidence:** high confidence that blind intervals exist;
moderate confidence in the chosen quality/cost trade until phase-swept comparisons run.
No promised speedup:
faithful analysis may be slower;
preaggregation bounds its memory.
**Visual risk:** expected restoration/brightening of previously undersampled transients, slab-boundary placement, first/last-window coverage, and differing live/offline time resolutions.

**Dependencies and gates:** sequential with SG2 if both edit offline feeding, and SG4 if sharing slab infrastructure.
The `ui/src/spectrum.rs` capped-hop/admission slice is disjoint from the currently inspected tuning files;
per-placement streaming moves layout resolution ahead of precompute in `offline/src/render.rs` and requires replay/composition coordination, as do expansions into take/configuration.
Measure 3/10/30-minute windows at all FFT sizes, 48/96/192 kHz and mono/stereo, with burst phases spanning the hop and tone/noise controls.
Include a compact set of one-taper Hann and maximum-five-taper sine-family phase and cost cases;
one FFT per taper changes both attenuation and work, so the existing one-taper probe cannot establish a universal policy.
Compare against an 8 ms analysis reference at the same slab width/configuration;
distinguish no blind spots from faithful peak capture.
Choose and document an attenuation tolerance from those comparisons before shipping any approximate policy.
For raw columns, verify admission estimates against retained/requested memory and exercise the declared out-of-budget behavior.
For streaming, bound data by admitted distinct output grids plus explicit margins and scratch;
test two non-nested placement widths with a transient just beside a boundary only one has.
Both alternatives must cover trimmed windows, first/last observations and deterministic output.
This is a rendering-coverage defect, separate from #310's optional multi-resolution DSP.

### SG4 — reduce CPU handoff and upload staging churn in stages

**Problem/evidence:** M/S:
ordinary changing runs allocate/copy about 1.3–2.9 MB per update and compare the entire visible byte run to choose 1–3 dirty slabs.
Prefix `Vec::drain` additionally shifts retained bytes when slabs leave the grid.
The current module's O(new columns) description characterizes tail folding, not the complete `window`/`view`/handoff path.
The fragment shader does not cause these CPU copies.

**A, smallest useful change:** skip the warm renderer's scratch allocation when dirty is empty;
for aligned slabs, write borrowed run slices directly, keeping a padded path for generic unaligned bin counts.
This removes one avoidable 3,828-byte allocation per callback and dirty-slab copies at the production shape.
Effort small, maintenance low, expected timing gain tiny/unmeasured, visual risk low.
Do not add a global staging pool just for this.
Its verification must reach `CallbackTrait::prepare` in the render crate:
use a targeted allocation probe around staging for zero and 1–3 dirty slabs, aligned production bins and generic odd bin counts, after GPU resources are warm.
Separate our staging requests from driver allocations;
the existing CPU scratch probe excludes this boundary and cannot measure A's benefit.

**B, smaller measured steps:** evaluate reusable CPU snapshot/scratch storage and deriving dirty keys from new, tail, first and held slabs while retaining the flat store and full recovery snapshot.
Measure those changes separately;
the dirty-range experiment may remove `SentRun::moved`'s full byte scan without requiring circular storage.
Conservative dirty sets still incur uploads, so compare CPU work and actual payload rather than assuming byte equality is free to replace.
`Arc::make_mut` alone can still copy the entire run while the previous callback holds it.
**C, only if B leaves material cost:** circular/keyed storage and immutable deltas, retaining a coherent full snapshot or regeneration route for first upload, dropped callbacks, resize, and GPU eviction.
Use this to address remaining prefix shifts/copies only after measuring them;
do not commit to a new packet architecture for theoretical savings.
Avoid changing shader math at the same time.

**Effort/maintenance:** medium for scratch reuse, medium–large for a snapshot/delta contract;
explicit generation/serial/slot invariants cost more to maintain than byte comparison.
**Benefit/confidence:** high confidence in reducing requested bytes/copies, moderate confidence in CPU time reduction in long/wide/multiple-surface views;
the entire measured stage is the upper bound on savings at those fixtures.
**Visual/correctness risk:** a stale lap after a dropped callback, pruning an interior MAX, losing held-copy repair, and missing a first-slab dirty write when only time advances.

**Dependencies and acceptance:** SG1 first for the same CPU module;
A can proceed independently in `render/src/spectrogram.rs` after overlap checks.
B can precede integrated #617 if kept to `ui/src/spectrogram.rs` and `render/src/spectrogram.rs` with unchanged public musical/history inputs.
Sequence B/C within those modules, and coordinate SG3 if either changes shared folding.
Retain the current all-sequence GPU/full-upload equivalence, dropped-ack, rung hysteresis, coarse-tier, ring-wrap, negative-time, resize, and first-slab tests.
Run cold/warm/no-new-columns comparisons with two surfaces of different sizes.
Adopt a more complex packet architecture only if repeated stage profiles show a meaningful CPU/allocator reduction and no regression in recovery cost;
otherwise stop after the smaller measured steps.

### SG5 — make surface lifetime and retention deliberate

**Problem/evidence:** S:
GPU eviction runs only from a spectrogram callback;
hiding the last one does not run that sweep.
CPU surfaces retain both folded history and sent snapshots until broader state destruction/reset, including SG1's oversized capacity.
This is retained memory with bounded surface IDs, not an established unbounded leak.

**Smallest useful change:** expose allocated/live bytes and document intentional retention;
release pathological excess after SG1 and provide an explicit frame/context teardown entry point for inactive GPU copies when memory pressure merits it.
**Larger alternative:** a surface owner separating history-fidelity state, upload knowledge, and GPU copies, with explicit active-surface notification at the frame boundary.
Do not create independent per-pane timers that cannot observe the last pane disappearing.

**Effort/maintenance:** medium;
an explicit lifetime contract is useful, but a generic eviction framework has an ongoing cost disproportionate to a 16 KiB LUT.
**Benefit/confidence:** high confidence in freeing measured/estimated retained memory;
no established frame-time benefit, and more frequent cold uploads can worsen reopen latency.
**Visual risk:** dropping fine aggregated slabs and rebuilding from coarsened history changes temporal detail;
retaining their CPU copy while dropping GPU buffers avoids that particular loss.
Never stop analysis just because the heatmap is hidden.

**Dependencies and gates:** GPU-only retention is independent of auto-tuning;
frame ownership hooks in `ui/lib.rs`, `ui/state.rs`, plugin shell, or offline composition require coordination with integrated #617 and shared lattice lifecycle work #645. Test one remaining surface, no remaining surfaces, repeated hide/show, context replacement, multi-pass discarded UI frames, and two differently sized placements.
Record allocated bytes before/after retirement and cold reupload time.
Specify which CPU temporal fidelity is preserved and require exact output for that scope;
any intentional loss needs explicit visual acceptance.
Do not duplicate the lattice history allocator or assume its capacity/reseed handshake is this spectrogram serial protocol.

### SG6 — validate attribution before GPU or overlay architecture changes

**Problem/evidence:** H/S:
#519's historical numbers and the new offline differential cannot isolate current shader cost;
the live egui timer closes on an independent 1×1 attachment, the same structure the historical probe found could overlap on Metal.
This is a measurement concern requiring reproduction, not a newly measured timer failure.
The fallback counters count refolds/full uploads but not bytes copied, delta counts, or stage time.

**Smallest useful change:** use targeted temporary CPU scopes for fold/view/diff/LUT/callback staging and text/tessellation;
record bytes, slots, allocation count, and executed callback count per surface when evaluating SG4/SG5. For GPU timing, first prove a bracket responds to multiplied work and closes after a real dependency, using asynchronous readback and a validated timestamp mechanism.
Do not replace working beginning-of-pass timestamps with advertised end timestamps without checking actual values.
Coordinate measurement methods with #642, but validate the egui timer independently:
`EguiGpuTimer` in the vendored renderer and lattice `GpuTimer` in `render/src/lib.rs` are different owners and query sets.
Landing the lattice bracket change does not establish the spectrogram/egui bracket's validity.

**Larger alternatives, conditional:** prefilter a level grid or prefix sums for pitch reads;
a GPU texture/mipmap representation;
compute-assisted folds;
indexed/reused text overlay preparation;
pipelined whole-frame export readback.
Also consider avoiding per-frame live analysis for whole-song-only layouts, but only after auditing all pane consumers and measuring that specific waste;
whole-song spectral panes themselves hide the current curve.
Each requires a measured bottleneck and a separate comparison, not a bundled rewrite.
Prefix sums or mipmaps over raw bytes cannot reproduce per-bucket level/tilt/clamp followed by averaging;
a prefiltered level cache adds level and tilt invalidation and possible full-grid work on their drags.
Caching colored pixels reintroduces pitch/size/palette invalidation removed by #503. An asynchronous export readback ring changes a shared frame/encode pipeline and retains additional complete output frames;
it is not a heatmap optimization.

**Effort/maintenance:** small for a focused CPU probe, medium for validated GPU attribution, medium–large for alternate GPU representations or export scheduling.
**Benefit/confidence:** attribution confidence is the immediate benefit;
performance gain is Q until measured at reachable pixel counts and surface combinations.
**Quality risks:** narrow partial attenuation, time aliasing, intensity changes at resize/rung transitions, banding/dither determinism, nonlinear color interpolation, and label/shadow clarity.

**Dependencies and acceptance:** renderer-only investigations can run alongside tuning, but machine-intensive measurements run sequentially under comparable load.
Vendor egui timing, shared text/shadow code, render frame hooks, and offline `frames.rs` must be checked against both active workstreams.
Use complete spectral composition at 1×/2×, 512/1,024/2,816 pitch pixels and a reachable 4K output, live and whole-song, two surfaces, short/full pitch range, and empty/dense MIDI overlays.
Report CPU, GPU, allocator requests, retained bytes, and upload payload separately, with cold pipeline state distinct from warm steady state.
No experiment graduates on theoretical operation count or target-size savings alone.

## Implementation order and active-work overlap

1. SG2 leads on recurring export correctness; coordinate its actual caller and integration fixture with replay work. SG1 can start immediately in its separate module while that coordination happens.
2. SG3 whole-song coverage policy and phase-swept baseline, followed by a memory-admitted or streaming repair with its corresponding scope.
3. SG4 A can be an early independent small change; evaluate flat-store SG4 B after SG1, and only then consider circular-store C.
4. SG5 explicit retention only after the memory/reopen/fidelity trade is established.
5. SG6 GPU/overlay/export experiments only when attribution warrants them; its measurement gates may be used earlier wherever a decision needs evidence.

There is no semantic dependency on the complete adaptive-tuning feature.
The spectrogram consumes selected audio power/history and resolved drawing coordinates, not the tuning solver's assignment decisions.
Audio input selection, source-aware note overlays, canonical recovery, effective configuration, and frame ownership are real interface boundaries;
preserve their authority and keep analysis/rendering out of real-time tuning progress.

Inspected 2026-09-05:
#636 `f6f1bdeb`, #637 `ee330330`, #639 `ddeeb507`, and #653 `152bf183`, including the cumulative three-dot diff from main and the active canonical-recovery worktree's dirty files.
The latter also had `core/canonical.rs`, `offline/replay.rs`, `plugin/editor.rs`, and `take/lib.rs` in progress.
Source identity/effective configuration touch `ui/lib.rs`, `ui/state.rs`, spectral composition/notes/names/roll, replay/render/frames, plugin shell/background, and test harnesses.
They do not currently edit `ui/src/spectrogram.rs`, `ui/src/spectrum.rs`, `render/src/spectrogram.rs`, or its WGSL.
That is an observed write set, not a promise about future changes.

| Proposed work | Tuning coordination | Lattice coordination |
|---|---|---|
| SG1 and SG4 CPU slab work | Can start now, preserve interfaces; no blanket #617 wait | Mostly disjoint from #643, but one writer within spectrogram slab code |
| SG4 A upload staging | Can start now | Dedicated render/spectrogram module is disjoint from lattice lib/WGSL work |
| SG3 capped-hop/admission slice | Can start in dedicated spectrum module; recheck test-file overlap | Independent until it changes shared frame ownership |
| SG2 offline caller and broad SG3 replay integration | Sequential with active #617 replay/render work, or explicitly coordinated slice | Shared offline acceptance fixtures may overlap |
| SG5 last-hidden-view/frame lifecycle | Integrated #617 is the practical coordination boundary for broad shell/state edits | Coordinate shared frame retirement with #645; different history semantics |
| SG6 egui timers/text/frame/export work | Recheck vendor/frames/spectral overlay paths | #662 edits lattice timing and shared text; egui timing has a separate owner and validation gate |

The lattice plan merged as #652;
implementation PRs appeared during the independent reviews.
The final overlap check inspected #661 `943c8efb` (bounded trail scratch, `scene/src/trail.rs`) and #662 `b8ecafb6` (lattice GPU timing/depth/bloom, `render/src/lib.rs`, shared `text.rs`, lattice/text WGSL and renderer tests).
#662 changes shared text pipeline/depth/bloom interfaces, so SG6 text work must coordinate with that actual implementation;
the dedicated spectrogram upload/shader modules remain disjoint.
#662 does not edit the vendored egui timer.
These draft implementations are not accepted performance measurements for this audit.
The final tuning recheck found #653 at `f548b13a` with the same relevant cumulative overlap and no dedicated spectrogram-module edits.
The canonical-recovery, lattice-GPU and trail worktrees were clean at that recheck;
the earlier dirty-file observation above records what was in flight at the initial snapshot.
Recheck current branches, PR heads, and dirty write sets before starting any implementation;
each mutating stream owns a separate managed worktree.
An audit subagent is read-only and shares no mutation stream.

## Rejected or deferred ideas

| Idea | Disposition and evidence |
|---|---|
| Reintroduce CPU-colored textures, gesture-resolution tiers, or general `SpectrumConfig` keys | Reject: restores the size/zoom/color recomposition and brightness hazards already closed by #355/#491/#503. |
| Use raw-byte mipmaps/prefix sums as an exact replacement | Reject the equivalence claim: averaging before level/tilt/clamp is a different operator. Keep only a measured level-space experiment with its additional invalidation costs. |
| Reduce storage to fewer buckets, halve live slab cap, or lower hidden analyzer rate | Defer: changes resolution/retention/continuity, not a free optimization. Requires paired core history and analysis guarantees. |
| Promote bytes to float/R16 for visual precision | Defer: existing half-dB encoding was compared visually; no measured artifact here justifies 2×/4× storage. |
| Share grids across every surface | Defer: different depth/span/rungs need different folds. Start with per-surface ownership; share immutable equal products only after measured duplication matters. |
| Drop all CPU state on hiding and rebuild later | Reject as a picture-preserving optimization: history tiers cannot recreate every previously folded fine slab. |
| Always move analysis to a worker or move folding to compute | Defer: currently renderer cost is not established as the dominant frame cost; a worker adds scheduling/snapshot/backpressure and live/offline coordination. Keep musical progress independent of GUI either way. |
| Zero padding, multiresolution/constant-Q, median filtering, ramp dither, scalloping correction | Existing owner [#310](https://github.com/yan-h/harmonigraph/issues/310). Do not duplicate it. Its original order-4/bilinear description is historical; audit each remaining idea against today's clamped-level shader. Multiresolution needs per-band timing; a symmetric median can remove short events and cannot be justified solely by overlapping windows. Dither must be deterministic and evaluated at the actual output quantization. |
| Repeat old FFT/reassignment projects | #401 and #342 are closed; inspect their shipped implementations before suggesting work. This audit makes no new DSP-speed promise. |
| Replace text/overlay caches broadly | Defer until CPU stages show the cost; source-aware names/roll are actively changing under #617, and existing shared text/shadow infrastructure already owns those resources. |

## Common verification and handoff contract

Picture-preserving changes use both existing spectrogram GPU reference tests and the offline spectral goldens, including tall, short, zoomed, whole-song and mixed-shadow scenes.
Run sequential-image fixtures for smooth scrolling, slab/ring boundaries, first-slab pruning, dropout, context loss, missed prepare, hidden/reopened surfaces, and multiple placements;
single goldens cannot prove those contracts.
Exercise per-bucket clipping, tilt extremes, quiet gradients, sharp ridges/noise, narrow/full pitch ranges, all four orientations, and gamma/sRGB paths where the changed code reaches them.
Keep deterministic output and label/readability acceptance separate from numerical level-space comparisons.

Report every intended picture change in its implementation PR, particularly SG2/SG3. Do not bless goldens as a substitute for comparing the changed output.
For an experiment, a measured rejection with the fixture and eliminated explanations recorded in its issue is a valid result.
This documentation-only task requires no release build and never swaps the shared DAW plugin slot.
Implementation PRs that touch the picture must build both plugin and offline renderer before pausing.

## Independent review record

Both independent initial reviews used the same complete proposal commit `994fbac5657193d0e0b14706808edc27c9807a9c`, component briefs and evidence against source `70e9f48d`.
Claude Opus/xhigh (`fix=false`) accepted with corrections to SG1/SG3 and priority;
Astra supported the direction with three SG3 contract corrections.
Neither received the other's report before its initial verdict.
[Reports and individually validated dispositions](evidence/spectrogram-audit/review-disposition.md) preserve every finding, accepted correction, qualification and declined dependency claim.
The revised plan and issues are coordinator-validated documentation changes;
they were not submitted for another independent verdict.
Draft PR [#663](https://github.com/yan-h/harmonigraph/pull/663) is open and not merged.
