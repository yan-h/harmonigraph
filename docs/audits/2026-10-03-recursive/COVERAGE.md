# Coverage index

Deep means sufficient evidence for the named question, never universal subsystem correctness.
Detailed scope, investigator, confidence and uncertainty remain in coverage.json.

| Leaf | Question | Status | Conclusion |
| --- | --- | --- | --- |
| A1 | Audio/MIDI callback bounded work, allocations and payload lifetime | deeply examined | Finite probe confirms discarded epochs bypass accepted-event budget: 3072 popped/0 retained versus matching 2048/2048 |
| A2 | Analyzer reconfiguration and sample/channel transitions | deeply examined | Existing size/count/rate ownership and discontinuity resource retention are precise; no new cache justified |
| A3 | Adaptive assignment and tuning changes × held notes × naming | deeply examined | Fixed actual Just E names E- under Just and falls back to E under current equal temperament with no matching node |
| S1 | UI edits × host automation × restored state | deeply examined | Restore adopts before input and camera remains host-owned; 20ms fallback limits editor-lock wait only |
| S2 | Scene derivation, normalized views, derived-state ownership | deeply examined | Recent normalization/range refactors remove duplicate work; current keys contain the inspected dependencies |
| S3 | Hidden/closed UI, resizing and resource lifecycle | deeply examined | Shared input/history and separate surface motion/GPU state earn their lifetimes; hidden replay is bounded |
| R1 | Lattice rendering, labels and atlas resource lifetime | deeply examined | Shared sheets already exist; text and lattice update timing must remain different |
| R2 | Stars, spectrogram, shadows and glow work/caches | deeply examined | Uniform split crosses real size threshold; Solo hidden-layer work preserves accepted color history |
| R3 | Scene changes × GPU resource invalidation | partial | Halo-only change replaces unchanged source; CPU prepare median delta 252.938us across 30 alternating pairs; extra ownership is not justified by this evidence |
| T1 | Recording × stop/seek/loop and canonical ordering | deeply examined | Producer/configuration/source closures protect distinct late-data frontiers and should remain separate |
| T2 | Live × replay × offline and appearance capture/export | deeply examined | Captured appearance and canonical ordering have coherent owners; malformed selected appearance is rejected |
| T3 | End-to-end encoding and readback performance | partial | Three export workloads completed, but pooled wall ranges vary up to 5.3–108.2s for the same workload; no pipeline speedup established |
| D1 | Build/test/development material maintenance cost | deeply examined | No new build architecture is justified; cache reuse requires matching Cargo feature selection |
| C1 | Independent challenge of major proposals and clean claims | deeply examined | Claims narrowed to actual mechanisms and fixture reach; no demonstrated host stall or free resource-split win |
| A4 | Finite-input numerical spectrum and history conservation | deeply examined | Inspected arithmetic and fixtures coherent; retain accepted Fast/Fold tradeoffs |
| A5 | Recovery from malformed nonfinite audio | deeply examined | Reproduced: 2564 display buckets stay nonfinite after two clean windows while newest history is finite and lit |
| A6 | Realtime payload ownership and actual CLAP allocation fixture reach | deeply examined | Fixed-value adoption and off-audio heap lifetime retain valid independent owners; current drain snapshots are bounded |
| R4 | Shadow and glow sampling at real shape/scale boundaries | deeply examined | Existing shadow/glow structure and support-scaled bounds earn their place; no new rework justified |
| T4 | Frame cadence and trimmed export semantics | deeply examined | Events and sample-time history have contracts; arbitrary pixels do not; --start intentionally starts audio history cold |
| H1 | Native Bitwig integration and latency | blocked | Deliberately not executed in active user session |
| D2 | Other platforms, hosts and device families | unexamined | Known Linux manifest issue linked; no new cross-platform claims |
| A7 | Does CLAP need a retained full processing-status enum? | deeply examined | Sole retained consumer needs u32; native ProcessStatus is 24 bytes and its AtomicCell is not lock-free; u32 cell is lock-free |
| P1 | CPU cost of shared visual runtime with dock, preview and no drawing | deeply examined | Median of run p50: active dock .675ms, dock+preview .906ms, active no-draw .064ms; no additional scene-cache owner justified by these costs |
