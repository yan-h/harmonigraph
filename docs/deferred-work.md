# Deferred work

Items that were evaluated and consciously parked —
not abandoned.
Each entry carries enough context (state, the actual work, the catch, and a value/effort read) to pick it up cold later.
Nothing here blocks anything today.

## Adaptive-tuning alternatives

**State.** Project-wide adaptive retuning is designed but not implemented.
The active design is documented in [`adaptive-tuning.md`](adaptive-tuning.md):
one lightweight tuner per independent note path, automatic aggregation into one full Harmonigraph, and fixed-delay central sequencing with sequential assignment.
Each new assignment sees its predecessors across tracks, and its correction remains frozen through release.
A missed assignment deadline delays the pending attack and reports a failure;
the affected track can remain late until a safe idle boundary.
Stop/Reset cancellation and a visible emergency stop at required-storage exhaustion are accepted exceptions to retaining pending attacks.
The active design records those decisions and their remaining implementation mechanics.

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
