# Production Metal asset validation

The [production workflow](../../metal-shader-assets.md) owns the reproduction commands and interpretation.
The embedded corpus is in `crates/harmonigraph-metal-assets/assets/` with its compiler provenance and hashes.
This record uses the actual installed provider and embedded bytes, with no runtime sidecar override.

`validation.txt` preserves test outcomes and raw-log hashes from the initial generator and expanded metadata/fallback run.
The native results contain all twenty fresh-process samples: five alternating source/strict pairs per cache condition, with three window openings per process.
Every process completed all three openings, dropped all three editor states, and created exactly one device.
The binary hash is in `native-provenance.json`; the source state was `4341b32e`, built before that commit was created.
The normal source and strict modes run the same executable.

The GPU results contain eight alternating pairs after one unmeasured warmup pair.
Each process runs the existing `timing::a_frame_` tests with `PROBE_FRAMES=240`: four tests report eight workloads, including two repeated Distance reference views.
The initial scratch runner expected six workloads and rejected its first warmup run; the recorded series follows that count correction.
Every accepted process reports positive library loads and zero source calls in strict mode, or the reverse in source mode, with zero load failures/rejections.
`gpu-provenance.json` identifies the same executable used for both modes.
Its source is `4341b32e` plus the committed timing-probe initialization/statistics instrumentation; the renderer behavior and assets are the same.
Rows marked `warmup` remain in the data and are excluded from reported medians.
The results retain all per-run medians and p10/p90 spreads.
GPU timestamps cover the prepare encoder and exclude the final egui composite; this is not a complete display-frame benchmark.

First/full times are observed on the UI update after the corresponding paint callback, following the preceding renderer invocation's submit/present.
They include up to one subsequent update interval and exclude outer-host/process launch.
They do not measure display photons, actual Bitwig audio contention, or natural long-idle cache eviction.

For the cache-blocked condition only, prefix the native executable with `/usr/bin/sandbox-exec -f PROFILE`.
The exact local profile was:

```scheme
(version 1)
(allow default)
(deny file-read* file-write*
  (subpath "/private/var/folders/3f/bjndnzhs0t50k9qpl1hpm_l40000gn/C/com.apple.metal"))
```

That cache directory is machine-specific and must be resolved on another machine.
The restriction does not delete caches and does not guarantee every compiler-service cache is cold.
The first source outlier remains in the data.
Keep these measurements separate from the historical copied-backend probe.
