# Compiled lattice resources (#647)

`CompiledLatticeResources` owns pipelines, layouts, samplers and immutable fallback textures/bindings.
It derives `Clone`, which retains GPU handles.
`LatticeResources` owns that compiled value plus the context's font/mark atlas publications, SDF identity, pane map, GPU timer and development shader watcher.
Its constructor initializes those mutable fields once.
The old `for_context` manual clone/reset list is gone.

`LatticePipelineCache` retains the compiled value directly.
The background startup job produces that same type, so startup no longer creates a throwaway context or allocates a timer only to discard it.
The existing cache mutex remains confined to initialization/reopening;
there is no new shared mutable owner, lock, heap indirection or compiled-handle clone on a steady frame.
The cache key remains instance identity, device identity and target format.
Each is required for compiled-object compatibility;
camera, viewport size, atlas publication, pane IDs and temporal history do not affect compilation.
No key was narrowed, and no previously unreachable history carry path is introduced.

Hot reload retains its existing cache bypass and context-local pipeline replacement.
A context owns a plain compiled value, so replacing its handles cannot replace handles held by another context.
The watched modules are still validated together before rebuilding;
text/common source publication to the other callback renderers is unchanged.
There is no shader, draw-plan, history-advancement, persisted-state or live/offline API change.

## Validation

The existing GPU fixtures exercise the new owner through production preparation:

- `reopening_reuses_pipelines_with_fresh_window_resources` creates real pane ink history, publishes both atlases and a nonzero SDF identity, then constructs a second context from the compiled owner while the first remains populated.
  Compiled pipeline and fallback texture identities are retained;
  atlas state, pane state and timer query/staging/notification resources are independent.
  Reopening with the same pane ID preserves exact pixels.
- `startup_worker_preserves_pixels_and_reuses_its_completed_pipelines` checks synchronous/background pixel parity, completed-worker reuse and an obsolete format result.
- `pipeline_cache_rebuilds_for_another_device_or_format` covers all key inputs, including colliding native device IDs from distinct instances.
- `a_reload_rebuilds_and_draws_both_bloom_variants` draws a real reload and verifies that another context's cloned compiled handles retain their original identities.
  A subsequent invalid edit retains the accepted pipeline identities and exact pixels.
- Existing target/history and two-pane timer fixtures retain their coverage.

Performance expectation is neutral steady-frame cost.
This extraction removes an ownership maintenance contract;
it makes no FPS or steady-frame speedup claim.

## Measurement method

The existing `lattice_tests::timing::a_frame_of_names_at_each_kernel_costs_this_much` probe supplies four workloads:
Gaussian/Distance shadows at the live-view and maximum shadow widths, with 30 names on 355 lit nodes at 768×768.
It discards ten warmup frames and records 120 frames per workload.
CPU scope is the complete callback `prepare`, including staging and encoding;
GPU timestamps bracket that preparation encoder, excluding the final egui composite.

Baseline and candidate test executables are preserved separately, so accepted paired runs need no intervening compilation.
They use the repository's optimized test profile and the same Metal adapter.
These are headless preparation measurements, not a Bitwig frame-rate benchmark.

A separately compiled scratch variant wraps the Rust system allocator with thread-local counters enabled only around `prepare`.
It records allocations/reallocations and requested bytes on the eleventh frame, after the ten warmups, using the same four workloads.
Requested bytes are the sum of allocation requests, not retained memory or GPU traffic;
native Metal allocations are not intercepted.
The counting executable's timings are excluded from CPU comparisons.
The scratch allocator and instrumented fixture remain verification artifacts in `/private/tmp/compiled-lattice-alloc.rs` and `/private/tmp/compiled-lattice-alloc-timing.rs`.

Initial validation passed 243 renderer tests (eight manual probes ignored), all five offline golden frames (two manual probes ignored), and ten hot-reload tests with real Metal pipelines.
All thirteen lattice golden frames remain unchanged.
No golden was blessed, and no GPU-adapter skip was used as validation.
