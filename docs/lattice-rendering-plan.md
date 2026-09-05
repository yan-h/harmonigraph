# Lattice rendering: maintenance and performance plan

## Status and scope

Tracking issue:
[#643](https://github.com/yan-h/harmonigraph/issues/643).
This records the audit and prioritization at `8d1ce70b` on 2026-09-05;
the component issues hold implementation scope, reproduction details and acceptance criteria.
The sequencing and constraints incorporate the supported findings from the requested Claude Opus/xhigh and independent Astra reviews on the same date.
The documentation does not implement the renderer changes or approve a different look.

Preserve the core → scene → render boundaries and the shared live/offline picture.
The strongest architectural investment is explicit ownership of per-view history, viewport targets, frame data and rendering passes.
Memory reduction and maintenance benefits are better established than any combined frame-rate improvement.

## Priority by cost and benefit

Effort is relative, not an implementation-time estimate.
The order favors bounded changes with direct evidence before new caches or GPU algorithms.

| Order / gate | Component slice | Effort | Expected return | Visual acceptance |
|---|---|---|---|---|
| 1 | [#642 A: complete the total GPU bracket](https://github.com/yan-h/harmonigraph/issues/642) | Small | Measure all lattice preparation before pricing changes | Unchanged |
| 2 | [Cull glow-free blur instances](https://github.com/yan-h/harmonigraph/issues/641) | Very small | Reuse the strip stage's glow predicate to remove confirmed redundant work; timing gain remains uncertain | Unchanged; two scratch fixtures compared byte-exact |
| 3 | [#644 A: discard the unused depth store](https://github.com/yan-h/harmonigraph/issues/644) | Very small | Remove the requirement to preserve an unread attachment after the pass | Unchanged painter order |
| 4 | [#645 A: allocate/write bloom attachments only when needed](https://github.com/yan-h/harmonigraph/issues/645) | Small–medium | Avoid the nodes-only attachment and bloom chain while off, including redundant color writes/stores | Unchanged labels, node illumination and bloom on/off/on |
| 5 | [#644 B: remove the depth attachment and pipeline state](https://github.com/yan-h/harmonigraph/issues/644) | Small–medium | Certain allocation reduction and simpler pipeline state | Unchanged across production, reference and hot-reload pipelines |
| Early, independent | [#648 A: reuse CPU scratch buffers](https://github.com/yan-h/harmonigraph/issues/648) | Small–medium | Reduce allocation churn where measured; no retained musical answers | Unchanged scenes and frame payloads |
| After the first GPU batch; renderer-only scope can precede #617 | [#645 B: separate history ownership](https://github.com/yan-h/harmonigraph/issues/645) | Medium | Remove viewport-driven history transfer while preserving existing scene inputs and CPU row allocation | Preserve same-capacity resize carry and the current growth/reseed baseline |
| After B and coordinated with integrated #617 | [#645 C: retire hidden targets](https://github.com/yan-h/harmonigraph/issues/645) | Medium | Retire large resources while retaining small temporal history | Preserve release color and elapsed-time behavior through hiding/reappearance |
| After the first GPU batch | [Name uniform groups and validate layouts](https://github.com/yan-h/harmonigraph/issues/646) | Medium | Safer shader changes; essentially neutral runtime cost; needs component-level coverage as well as layout checks | Unchanged, including blit prefix layout |
| After #617, measured | [#648 B: consider musical caching/indexing](https://github.com/yan-h/harmonigraph/issues/648) | Medium–large | Only retain optimizations that repay their maintenance cost at actual aggregated workloads | Unchanged matching, ties, configuration and lifecycle behavior |
| As measurements need it | [#642 B: detailed stage timing and counters](https://github.com/yan-h/harmonigraph/issues/642) | Medium | Attribute residual cost with query allocation for the passes that execute | Unchanged; asynchronous timing remains isolated per owner |
| Conditional experiment | [Reuse convolution weights and normalization](https://github.com/yan-h/harmonigraph/issues/649) | Small–medium | Exploit the 64-entry circular kernel only if convolution remains material after culling | Numerical comparison; equivalent arithmetic need not round identically |
| Conditional experiment | [Evaluate lower-resolution glow](https://github.com/yan-h/harmonigraph/issues/650) | Medium | Reduce glow allocation/fill after cheaper attachment changes are measured | Explicit quality tradeoff in node interiors and glyph wash, as well as halos and motion |
| Last, conditional | [Evaluate compute convolution](https://github.com/yan-h/harmonigraph/issues/651) | Large | Only if simpler changes leave a measured convolution bottleneck | Compare numerical output, temporal carry and goldens |

The first GPU batch is 1–5, sequenced within one rendering stream.
The depth and optional-bloom slices share pipeline variants, so reuse their work and adjust their implementation order if removing depth first simplifies the attachment choice.
CPU buffer reuse can proceed early after a file-overlap check, without waiting for aggregation semantics to settle.
Keep the A/B/C slices independently reviewable within their existing component issues.

[Frame/pass extraction #647](https://github.com/yan-h/harmonigraph/issues/647) is a discipline within these changes, not a standalone file-restructuring project.
Extract responsibilities as implementation reaches them;
the benefit is ownership of inputs, resources and invariants rather than file movement.
Caching and GPU experiments are recorded options with measurement gates, not a requirement to ship every experiment.
A measured rejection is a useful completed experiment and should remain in its issue.

## What the architecture should make explicit

| Owner | Responsibility |
|---|---|
| Frame builder | Associate nodes, markers and labels once; produce an immutable ordered draw plan and payloads |
| Per-view state | Layout-dependent data and temporal history, including the agreement between CPU glow rows and GPU row contents |
| GPU resources | Pipelines, reusable buffers and texture allocation/retirement |
| Individual passes | Shadow cells/blur, ink history/convolution, glow, ordered scene composition and optional bloom |
| egui adapter | Schedule preparation and composite the finished view |

Before #645 B,
resize already preserved temporal ink history by taking the outgoing `Offscreen`'s strip and adopting it in `Offscreen::ensure_glow` when its row capacity matched.
The renderer now keeps that history in `PaneBuffers::ink_history` separately from viewport targets,
removing the transfer mechanism while preserving its behavior.
The CPU/GPU handshake remains capacity-based:
`GlowFade::step` asks every row to reseed with `mix = 1.0` when capacity changes,
while `PaneBuffers::ensure_ink_history` retains same-capacity strips and replaces them on capacity changes.
Discarding history at unchanged capacity gets no automatic reseed signal and can extinguish a release whose current ink has already disappeared.
Retain that fading color deliberately;
a new empty strip cannot reconstruct it from current ink alone.

Capacity growth has a different baseline from same-capacity resize:
the current implementation creates a new strip and reseeds every row from current ink, so a release with no current ink can lose its color on growth.
This follows from the source;
the review did not run a dedicated growth reproduction.
The ownership refactor in #645 B preserves this existing growth/reseed behavior.
Preserving old colors across growth would be a separately scoped visual change, with a fixture that crosses a capacity boundary during a glow-only release and explicit acceptance of changed output.

Target eviction must also define what happens when the last lattice view disappears;
an age counter advanced only by another lattice callback cannot observe that case.
Settle history ownership before adding eviction, including coordinated CPU/GPU retirement if history itself is ever discarded.
Preserve how `GlowFade::step` uses elapsed time on reappearance rather than implicitly freezing hidden time or restarting the fade.
The current surface identities bound the pane map;
the problem is retained large allocations rather than an established unbounded leak.

Keep each retained value keyed on its actual inputs.
Membership, view direction, tuning and dynamic envelopes are different dependencies;
one broad `ViewConfig` or frame-time key would erase the benefit or hide a carry-forward defect.
GPU uniform names and actual layout checks should make the transport contract explicit without changing persisted settings.
Offsets, types and strides do not catch a component transposition within a correctly laid-out vector.
Map renamed components to observable behavior, identify gaps in the existing goldens, and add focused coverage for uncovered behavior that the rename could change.
Preserve the blit shader's shorter uniform prefix, including the bloom-strength slot, or deliberately update and validate both bindings together.

The existing Metal timer documents working beginning-of-pass timestamps and unreliable end-of-pass/encoder writes.
Moving the total bracket keeps its two beginning timestamps and tail pass.
Detailed scopes need a sequence of beginning timestamps plus a tail, a per-frame map for the passes that actually execute, and appropriately sized query/resolve/readback storage.
Resolve only written queries and retain asynchronous readback and docked-view timer ownership.

## Dependency on adaptive tuning

**Do not wait for the whole adaptive-tuning feature to finish.** The [decided design](adaptive-tuning.md) keeps the pure musical policy in core and the companion free of the renderer.
Assignment never reads camera state, `ViewConfig` reach, display tolerance, render resolution or editor progress.
Renderer-local changes therefore do not depend on the completed [sequencer #616](https://github.com/yan-h/harmonigraph/issues/616) or [musical policy #621](https://github.com/yan-h/harmonigraph/issues/621).

There is nevertheless an active interface and file-overlap boundary at the [aggregation/configuration/recovery foundation #617](https://github.com/yan-h/harmonigraph/issues/617).
Checked on 2026-09-05:

| Tuning work inspected | Relevant overlap |
|---|---|
| [Source identity PR #636](https://github.com/yan-h/harmonigraph/pull/636), `f6f1bde` | Source-aware mark lookup in `scene/derive.rs`; changes to tracker/roll/history/take/replay; lattice/glow pane edits at this head are test API adaptations; renderer golden/mark fixtures also change |
| [Effective configuration PR #637](https://github.com/yan-h/harmonigraph/pull/637), `ee33033` | Moves effective tuning authority out of UI frame derivation; changes `ui/lib.rs`, `ui/state.rs`, configuration and replay |
| [CLAP boundary PR #639](https://github.com/yan-h/harmonigraph/pull/639), `ddeeb50` | Vendor boundary, CI and protocol documentation; no renderer production files |
| Active `codex/617-canonical-recovery` work after that head | Uncommitted core tracker/roll/canonical-state changes; the complete foundation is not settled merely because #636/#637 have draft heads |

This is a dated overlap check, not a permanent claim that an active branch will touch no additional files.
Recheck the current heads and active write set before beginning an implementation stream.

| Rendering scope | Earliest sensible start |
|---|---|
| Total timing, blur cull, depth discard/removal and optional bloom | Now, after the overlap check; preserve the current scene/event/configuration inputs |
| CPU scratch-buffer reuse | Now if confined to storage reuse with unchanged semantics; check `scene/derive.rs` and frame-builder overlap rather than assuming different crate names make work disjoint |
| Uniform cleanup and detailed timing | Independent of auto-tuning; sequence behind the first GPU batch or when a measurement needs more detail |
| Renderer-only history ownership (#645 B) | Can precede #617 after an overlap check if `Scene::glow_rows`, instance inputs, the CPU row allocator and `SharedState` stay unchanged; serialize with other renderer work |
| Hidden-view retirement (#645 C), UI/shared-state history changes and broad frame/shared-state extraction | Use integrated #617, including canonical recovery, as the coordination boundary; C also follows B, and #617 does not solve the renderer's row/history handshake |
| CPU caches and indexed musical matching | Start from integrated #617 inputs and remeasure the aggregated workload; no need to wait for #621's scoring constants |
| Convolution or resolution experiments | When their performance/resource prerequisites justify them; completion of auto-tuning is not their gate |

Rendering continues to consume actual emitted pitches and output times, source-scoped resets/recovery and coherent effective configuration.
An attack-time assigned node is metadata, not a replacement for the current emitted pitch after player expression or a configuration change.
Display cache keys must reflect their actual resolved musical inputs;
a global policy/configuration revision is not automatically an appropriate key for every visual value.
No rendering refactor may move musical state progression or policy execution onto a GUI callback.

The timing/cull/depth/bloom/uniform changes share `harmonigraph-render/src/lib.rs` and `shaders/lattice.wgsl`, so run them sequentially even though they are independent of adaptive tuning.
Every mutating stream still uses its own owner-managed worktree.
Integration must check both the renderer's picture and the tuning feature's editor-independent behavior;
large render/benchmark jobs also contend for the same machine, so performance comparisons should be run under comparable load.

## Measurements and their limits

The audit used release-mode headless probes on the local machine, not end-to-end Bitwig measurements.

| Fixture | Observed result |
|---|---|
| Existing UI-only picture probe, 1000 × 900 points, six held voices | Lattice median 0.039 ms; excludes GPU callback preparation and execution |
| Existing full-GPU-prepare probe, 768 × 768 pixels, 355 lit nodes and 30 synthetic names | Medians 1.244–2.483 ms across shadow kernels/widths, with substantial sample variation; details in #642 |
| Scratch CPU fixture, 273 nodes / 10 voices | Derivation 0.017 ms; callback construction without labels 0.036 ms |
| Scratch CPU fixture, 14,877 nodes / 64 voices | Derivation 2.034 ms; callback construction without labels 0.625 ms |
| Scratch blur cull, 225 shipped audio-ring nodes with zero or one MIDI glow | Zero differences in each final 768 × 768 RGBA frame; timing gains were small/noisy; details and reproduction in #641 |

The existing probe commands are:

```sh
cargo test --release -p harmonigraph-render a_frame_of_names_at_each_kernel -- --ignored --nocapture --test-threads=1
cargo test --release -p harmonigraph-ui profile_picture_panes -- --ignored --nocapture --test-threads=1
```

For a 3840 × 2160 native-pixel lattice pane at render scale 2:

| Allocation | Format-derived size | Proposed saving |
|---|---|---|
| Main scene color | 253.1 MiB | Retained; required for the picture |
| Unused depth | 126.6 MiB | Entire allocation after removal; discard its store as an earlier slice |
| Nodes-only bloom input | 253.1 MiB | Entire allocation while bloom is off |
| Bloom intermediates at native-size fractions | 23.7 MiB | Entire allocation while bloom is off |
| Glow field at scene resolution | 253.1 MiB | Half width/height would save 189.8 MiB, subject to visual acceptance |

The nodes-only attachment is also cleared, drawn and stored while bloom is off, including a full-screen glow composition when enabled.
Optional bloom therefore avoids color-attachment work as well as retained memory.
Changing the unused depth store to `Discard` removes a preservation requirement before the larger pipeline cleanup.
These are format-derived allocation estimates and identified write/store operations, not measured physical memory traffic or frame-rate predictions;
compression, tiling and driver behavior affect actual bandwidth and timing.
Do not infer that these stores dominate the frame without measuring it.

Lower-resolution glow changes a deliberate texel-aligned design in `GlowTarget::new`:
node ink and glyph fills read the scene-resolution light field, so reconstruction changes their interior illumination as well as the outer halo.
Preserving the node/label wash is the first quality gate, followed by weak gradients, color boundaries and motion.
Half width/height gives 75% fewer pixels in that target, not 75% less work in the complete renderer.

After aggregation, price CPU work against the approximately 15 simultaneous notes across three sources described in #617 and its admitted session limit, currently 256 held voices.
Use sufficiently large windows and actual source/configuration/recovery transitions rather than extrapolating the small default frame.
Held voices do not bound glowing nodes:
`derive_scene` lights every matching lattice position, and `GlowFade` retains releasing nodes after their current ink disappears.
The current glow row allocator caps rows at 4096, so measure actual lit-row counts after the cull rather than using 256 voices as a convolution-work ceiling.
That allocator bound is not evidence that ordinary scenes reach it.

The angular kernel has 64 distinct relative weights in exact arithmetic;
its normalization is shared by the 64 circular output columns, with a separate flat mean column.
Only the kernel normalization `lobes` is shared this way;
the ink-dependent `wsum` still varies with each row and output column.
Evaluate reuse of those weights and their normalization together before a compute rewrite.
Hoisting a normalization shared between fragments still needs a concrete precomputation/storage path, and floating-point reordering needs numerical verification.
Retain #651 as a last, measured option rather than dropping it on an invalid voice-count bound.

## Acceptance across components

Use the existing lattice and offline golden sets for picture-preserving changes.
Do not bless changed output merely to finish a refactor.
Convolution arithmetic changes require numerical comparison, and lower-resolution glow requires explicit visual assessment before adoption.

Static frames alone cannot verify resource or cache lifetime changes.
Exercise a fading glow with no current ink, row growth/reuse, resize and render-scale changes, feature toggles, hidden-view reappearance, simultaneous dock/preview views and offline replay.
Keep same-capacity resize, bloom toggles during a release, and capacity growth as distinct fixtures so preservation of the existing growth baseline is not mistaken for preserving color through growth.
Source/configuration work also needs independent same-channel/key sources, expression changes, source removal/recovery and coherent effective tuning.
Select focused fixtures for the behavior each component changes rather than adding a copy of every test to every PR.

Each implementation records its actual CPU/GPU/allocation effects and the limits of the measurement.
Keep painter order, each label's shadow/ink position, label exclusion from bloom and per-surface history behavior intact.
Plugin-visible changes retain the repository's normal two-package release build, loadable handoff and draft-PR requirements.
