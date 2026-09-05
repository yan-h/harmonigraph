# Lattice rendering: maintenance and performance plan

## Status and scope

Tracking issue:
[#643](https://github.com/yan-h/harmonigraph/issues/643).
This records the audit and prioritization at `8d1ce70b` on 2026-09-05;
the component issues hold implementation scope, reproduction details and acceptance criteria.
The documentation does not implement the renderer changes or approve a different look.

Preserve the core → scene → render boundaries and the shared live/offline picture.
The strongest architectural investment is explicit ownership of per-view history, viewport targets, frame data and rendering passes.
Memory reduction and maintenance benefits are better established than any combined frame-rate improvement.

## Priority by cost and benefit

Effort is relative, not an implementation-time estimate.
The order favors bounded changes with direct evidence before new caches or GPU algorithms.

| Priority | Component | Effort | Expected return | Visual acceptance |
|---|---|---|---|---|
| 1 | [Complete GPU timing](https://github.com/yan-h/harmonigraph/issues/642) | Small | Measure all lattice preparation; add detailed scopes/counters where they answer a current question | Unchanged |
| 2 | [Cull glow-free blur instances](https://github.com/yan-h/harmonigraph/issues/641) | Very small | Remove confirmed redundant work with one consistent eligibility rule; timing gain remains uncertain | Unchanged; two scratch fixtures compared byte-exact |
| 3 | [Remove unused depth](https://github.com/yan-h/harmonigraph/issues/644) | Small–medium | Certain allocation reduction, fewer writes and simpler pipeline state | Unchanged painter order |
| 4 | [Separate history/target lifetimes](https://github.com/yan-h/harmonigraph/issues/645) | Medium–large | Main architectural investment; optional bloom allocation and hidden-view retirement reduce retained memory | Unchanged through resize, toggles, eviction/reappearance and releases |
| 5 | [Name uniform groups and validate layouts](https://github.com/yan-h/harmonigraph/issues/646) | Medium | Safer shader changes and less packing ambiguity; essentially neutral runtime cost | Unchanged |
| 6 | [Extract frame construction and passes](https://github.com/yan-h/harmonigraph/issues/647) | Medium–large | Local responsibilities and explicit ownership; extraction alone promises no speedup | Unchanged ordering, composition and history |
| 7 | [Reuse CPU buffers; measure caching/indexing](https://github.com/yan-h/harmonigraph/issues/648) | Small–large | Buffer reuse first; narrow caches and matching indexes only where large-window measurements repay their maintenance cost | Unchanged matching, ties, configuration and lifecycle behavior |
| 8 | [Precompute convolution weights](https://github.com/yan-h/harmonigraph/issues/649) | Small–medium | Conditional experiment if the measured convolution cost is material | Compare numerical output; equivalent arithmetic need not round identically |
| 9 | [Evaluate lower-resolution glow](https://github.com/yan-h/harmonigraph/issues/650) | Medium | Conditional memory/fill saving in the glow field; requires coordinated sampling changes | Explicit quality tradeoff, including motion; visual acceptance before adoption |
| 10 | [Evaluate compute convolution](https://github.com/yan-h/harmonigraph/issues/651) | Large | Conditional only if simpler changes leave a measured bottleneck | Compare numerical output, temporal carry and goldens |

The first implementation batch is priorities 1–3, sequenced within one rendering stream.
Priority 4 is the next substantial investment, followed by 5. Extract the responsibilities in 6 as work reaches them;
the useful change is ownership of inputs, resources and invariants, not file movement by itself.
Priorities 7–10 are recorded options with measurement gates, not a requirement to ship every experiment.
A measured rejection is a useful completed experiment and should remain in its issue.

## What the architecture should make explicit

| Owner | Responsibility |
|---|---|
| Frame builder | Associate nodes, markers and labels once; produce an immutable ordered draw plan and payloads |
| Per-view state | Layout-dependent data and temporal history, including the agreement between CPU glow rows and GPU row contents |
| GPU resources | Pipelines, reusable buffers and texture allocation/retirement |
| Individual passes | Shadow cells/blur, ink history/convolution, glow, ordered scene composition and optional bloom |
| egui adapter | Schedule preparation and composite the finished view |

Temporal ink history must not be owned by a viewport-sized texture that resizing destroys.
Target eviction must also define what happens when the last lattice view disappears;
an age counter advanced only by another lattice callback cannot observe that case.
Preserve small history deliberately or coordinate its reset/reseed across CPU and GPU.

Keep each retained value keyed on its actual inputs.
Membership, view direction, tuning and dynamic envelopes are different dependencies;
one broad `ViewConfig` or frame-time key would erase the benefit or hide a carry-forward defect.
GPU uniform names and actual layout checks should make the transport contract explicit without changing persisted settings.

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
| Complete timing, blur cull, depth removal and uniform cleanup | Now, after the overlap check; preserve the current scene/event/configuration inputs |
| Optional bloom or other GPU-only lifetime changes | Can proceed independently when confined to GPU resources; coordinate with the larger lifetime refactor |
| Combined history/view lifecycle refactor and broad frame/shared-state extraction | Start from the integrated #617 foundation, including canonical recovery, rather than changing its live interfaces concurrently |
| CPU caches and indexed musical matching | Start from integrated #617 inputs and remeasure the aggregated workload; no need to wait for #621's scoring constants |
| Convolution or resolution experiments | When their performance/resource prerequisites justify them; completion of auto-tuning is not their gate |

Rendering continues to consume actual emitted pitches and output times, source-scoped resets/recovery and coherent effective configuration.
An attack-time assigned node is metadata, not a replacement for the current emitted pitch after player expression or a configuration change.
Display cache keys must reflect their actual resolved musical inputs;
a global policy/configuration revision is not automatically an appropriate key for every visual value.
No rendering refactor may move musical state progression or policy execution onto a GUI callback.

The timing/cull/depth/uniform changes share `harmonigraph-render/src/lib.rs` and `shaders/lattice.wgsl`, so run them sequentially even though they are independent of adaptive tuning.
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

For a 3840 × 2160 native-pixel lattice pane at render scale 2, texture formats imply approximately 126.6 MiB for unused depth, 253.1 MiB for the bloom-only color attachment and 23.7 MiB for bloom intermediates.
The first saving is unconditional if depth is removed;
the latter two apply while bloom is off.
These are allocation estimates excluding driver overhead, and are not proportional frame-rate predictions.
Half width/height glow would have 75% fewer pixels in that one target, not 75% less work in the complete renderer.

After aggregation, price CPU work against the approximately 15 simultaneous notes across three sources described in #617 and its admitted session limit, currently 256 held voices.
Use sufficiently large windows and actual source/configuration/recovery transitions rather than extrapolating the small default frame.

## Acceptance across components

Use the existing lattice and offline golden sets for picture-preserving changes.
Do not bless changed output merely to finish a refactor.
Convolution arithmetic changes require numerical comparison, and lower-resolution glow requires explicit visual assessment before adoption.

Static frames alone cannot verify resource or cache lifetime changes.
Exercise a fading glow with no current ink, row growth/reuse, resize and render-scale changes, feature toggles, hidden-view reappearance, simultaneous dock/preview views and offline replay.
Source/configuration work also needs independent same-channel/key sources, expression changes, source removal/recovery and coherent effective tuning.
Select focused fixtures for the behavior each component changes rather than adding a copy of every test to every PR.

Each implementation records its actual CPU/GPU/allocation effects and the limits of the measurement.
Keep painter order, each label's shadow/ink position, label exclusion from bloom and per-surface history intact.
Plugin-visible changes retain the repository's normal two-package release build, loadable handoff and draft-PR requirements.
