# Deferred work

Items that were evaluated and consciously parked —
not abandoned.
Each entry carries enough context (state, the actual work, the catch, and a value/effort read) to pick it up cold later.
Nothing here blocks anything today.

## Adaptive-tuning alternatives

**State.** Project-wide adaptive retuning is built.
[`adaptive-tuning.md`](adaptive-tuning.md) is the design and the account of what shipped:
one lightweight tuner per independent note path, automatic aggregation into one full Harmonigraph, and central sequencing at a chosen fixed delay with sequential assignment.
Each new assignment sees its predecessors across tracks, and its correction remains frozen through release.
A missed assignment deadline delays that one attack and reports it;
unrelated ready notes on the same track keep their schedule.
Stop/Reset cancellation and a latched terminal fault at required-storage exhaustion are the accepted exceptions to retaining pending attacks.
What follows is the set of alternatives that were considered and parked, which is why this entry survives its own implementation.

**Immediate and jointly optimized alternatives.** Independent immediate assignment from prior snapshots permits simultaneous cross-track notes to miss each other's choices.
That does not meet the chosen musical requirement.
Immediate shared-state serialization instead needs a real-time contention and event-ordering contract across host callbacks.
The central sequencer accepts a measured fixed normal delay to keep one policy owner and complete chronological context.
Joint chord optimization is unnecessary for that requirement:
deterministic sequential assignment is sufficient to let each attack see its predecessors, without promising a unique absolute comma placement.
None of these alternatives is an additional launch mode.

**Reconciliation.** Adaptive movement of already-sounding voices is excluded absolutely by Yan's decision.
It is not a deferred tuning mode, listening experiment or deadline-failure recovery option.

**Compatibility outputs.** MTS-ESP is useful for a conventional global note/channel tuning table, but it does not naturally carry Harmonigraph's per-voice frozen adaptive assignments or collect note lifecycle.
MPE requires channel allocation and bend-state recovery;
VST3 adds another host and instrument interoperability matrix.
Bitwig itself converts a note effect's per-note pitch to MPE or VST3 note expression for the instrument downstream, so the CLAP-only output already reaches non-CLAP instruments inside Bitwig.
The initial implementation targets the actual personal environment —
macOS, Bitwig and compatible CLAP instruments —
and adds another backend only for a concrete instrument that needs it.

**Process and product variants.** A headless conductor, cross-process shared-memory transport, one full-plugin class that switches between tuner and hub roles, and a central MIDI rack all add lifecycle or routing surface without improving the intended workflow.
The Bitwig spike may reopen only the packaging or process boundary if separate companion instances cannot share an in-process session reliably.
The initial storage is bounded and in-process, with no pointer-free arena or memory-mapped ABI requirement.
A cross-process transport would need its own layout, ownership, synchronization and recovery design.

**Value / effort.** All are medium-to-high maintenance multipliers rather than cheap compatibility switches.
Reconciliation remains excluded;
revisit another alternative only when a concrete musical or hosting requirement justifies its cost.

## Depth-buffer sorting

**State.** The lattice renders through an offscreen colour pass with no depth attachment (`crates/harmonigraph-render/src/lib.rs`).
Occlusion follows the sheet and per-node painter order materialized by `LatticeCallback::from_scene`.
The previously unread `Depth32Float` attachment and its pass-through `Always` pipeline state were removed in the [first GPU batch](lattice-gpu-batch.md).

**The prerequisite.** Reproduce a concrete overlap artifact before proposing depth-based ordering.
Define which ring, marker, label and shadow should cover which other element, including across sheets, and compare that intended result with the existing painter order.

**The current geometry and composition.** Nodes are camera-facing billboards, not curved opaque surfaces with fragment-varying depth.
Their rings, glyphs, fades and shadows are composed in the scene's deliberate sheet and per-node order.
Node glow is assembled in a separate depthless pass and composited before the ordered scene;
it is also sampled to illuminate node ink and labels.
Enabling `Less` or `LessEqual` in the scene would change its ordering contract, but would neither add curved geometry nor make the existing glow field depth-aware.

Transparent ring and glyph fragments still require an explicit blending and depth-write policy.
There is no established opaque-core/transparent-skirt split for the current picture.
A proposal must specify its geometry, depth consumers and composition against this architecture, then verify the affected overlaps, fades, shadows and glow in live and offline output.

**Value / effort.** Unpriced until an artifact and candidate behavior are demonstrated.
There is no measured benefit supporting a depth-sorting refactor today.
**Do it only if a specific overlap artifact shows up in practice.** The [#644](https://github.com/yan-h/harmonigraph/issues/644) cleanup removes the unread allocation, clear, writes and store while preserving painter order.
The format-derived allocation saving does not establish physical bandwidth or frame-time savings.
Reintroducing depth would require a demonstrated consumer and composition rule, not infrastructure held for a hypothetical effect.

## Spectrogram follow-ups beyond the selected fixes

**State, 2026-09-06, settled.** Yan was satisfied with current performance and selected only SG1 bounded gap recovery, SG4A direct upload staging and SG2 offline timestamp repair, which landed together in [#710](https://github.com/yan-h/harmonigraph/pull/710).
Every issue below is closed as not planned.
The [spectrogram plan](spectrogram-rendering-plan.md) and [#654](https://github.com/yan-h/harmonigraph/issues/654) preserve the audit and current scope.
The following options are deferred and not planned, with no scheduled implementation, profiling campaign or periodic re-evaluation.
Reopening requires the stated concrete trigger and explicit reprioritization;
landing a prerequisite or having spare agent capacity is insufficient.

| Option | Evidence and reason to leave it parked | Reopening trigger |
|---|---|---|
| [SG3 / #657: whole-song temporal coverage](https://github.com/yan-h/harmonigraph/issues/657) | The missed-transient defect remains documented. A faithful fix may increase FFT work, and bounded streaming needs explicit placement-grid and memory contracts. | A concrete export requires the missing temporal detail; decide coverage, attenuation and memory together. |
| [SG4B/C / #670: CPU handoff/storage refactors](https://github.com/yan-h/harmonigraph/issues/670) | The measured warm stage is roughly 0.07–0.24 ms; explicit dirty/slot/snapshot state has an ongoing maintenance cost. SG4A shipped separately in #710. | A reproduced preparation bottleneck or necessary ownership change justifies a simpler improvement; circular storage additionally needs material residual cost after smaller steps. |
| [SG5 / #659: surface retirement](https://github.com/yan-h/harmonigraph/issues/659) | More eviction can lose folded temporal detail or increase reopen work. SG1 already owns pathological gap capacity. | Demonstrated memory pressure or a necessary lifetime change, with fidelity and reopen acceptance. |
| [SG6 / #660: dedicated attribution/GPU/text/export experiments](https://github.com/yan-h/harmonigraph/issues/660) | No measured shader/overlay bottleneck justifies a campaign or new representation. | A concrete performance or visual problem needs a targeted attribution decision; grow the experiment only from evidence. |

The original component measurements, reproductions and proposed acceptance criteria remain in their issues.
The focused checks that validated the three selected fixes were part of those fixes and are not a restart of SG6.

## Not deferred — closed

- **Render-style final trim.** **Done.** The aesthetic pass the entry asked
for was made, and none of the animated paints was kept:
Vortex, Checker and Spiral are gone along with the field machinery behind them (the noise, the sphere mapping, the per-node seed and the swirl gradient).
`NodeStyle` went with them rather than surviving as a one-variant enum, the way `OuterStyle` and `CoreStyle` did before it:
a blob's `node_style` is now an ignored unknown field, held by `a_persist_blob_naming_a_retired_node_style_still_loads`.
The disc they painted is gone too:
a node is its ring stack read out from an empty middle (`ring_inner`), and what lights that middle is the node glow, in the colours the node itself draws —
every layer contributing in proportion to the radial width it takes up, through the ink strip.
- **The piano roll's geometry** (was two entries here: baking settled notes
into cached meshes, then superseding that with a wgpu callback).
**Built.** The roll now draws as one instanced quad per note segment through `roll_paint_callback`, with a square-cornered box SDF painting the note's body and the two rim bands beside it as bands of one distance —
the reasoning, the measurement it came from, and why the instance buffer is still rewritten every frame all live in `crates/harmonigraph-render/src/roll.rs`'s module doc, which is where they belong now that there is code to read them against.
Mesh baking is dead rather than deferred:
it addressed the 0.5 ms of tessellation and none of the 4-5 ms of upload, and with the notes off egui's vertex buffer entirely there is nothing left for it to cache.
The `roll` row under `verts` in the performance overlay reports what the roll now costs (note count), since its geometry no longer passes through egui's vertex count.
- **Surface-format assumption** (`ASSUMED_SURFACE_FORMAT = Bgra8Unorm` in
`harmonigraph-plugin/src/editor.rs`):
the only clean fix needs `RenderState` access that lives upstream in egui-baseview, and upstreaming is off the table, so this stays as-is.
The constant is the knob if a mismatch ever panics on an exotic host.
- **Alternate skins / live re-skinning**: parked by choice.
