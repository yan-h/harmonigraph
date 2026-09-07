# One precompiled Metal pipeline

This experiment measures the horizontal shadow-blur pipeline from the production renderer.
The regular plugin and offline renderer continue to use WGSL unchanged.
The manual probe can instead load two precompiled `.metallib` stages through wgpu's public passthrough API,
then compare complete lattice frames with only that pipeline replaced.

The existing cold-start profile put this pipeline at about 260 ms,
including roughly 245 ms compiling its two Metal libraries.
It is a useful first case because it uses a texture, a sampler and one instance buffer,
without runtime storage arrays or pipeline constants.
It does not establish that every other shader can use passthrough with the same amount of work.

## Measured result

On the local macOS 26.6.2 machine, four fresh processes per mode and condition,
alternating both orders:

| Pipeline construction | WGSL, median | Precompiled, median |
|---|---:|---:|
| Metal disk cache blocked | 323.161 ms | 22.484 ms |
| Disk cache available and warmed | 1.058 ms | 0.365 ms |

Cold ranges were 272.436–400.236 ms for WGSL and 17.820–26.094 ms for precompiled libraries.
The cold median reduction was 300.676 ms, or 93.0%.
The two precompiled libraries total about 12.5 KiB;
loading them took 0.106–0.586 ms in the cold trials.
The remaining time was mostly native GPU pipeline-state creation.

Both full-frame parity comparisons passed byte-exactly,
and deliberately erasing the blur changed more than 100 pixels in each fixture.
The generated vertex and fragment source hashes also matched those captured from the production wgpu Metal backend during the earlier profile.
This is evidence for one pipeline on one machine, not a measured whole-editor speedup.
Steady-state GPU execution time was not benchmarked by this experiment.

The libraries were built with Apple Metal compiler `32023.883` using Metal 3.2,
fast math, preserved invariance and a macOS 15 deployment target.
The source, entry names, binary hashes and exact flags are recorded in the artifact's `manifest.json`.
The first artifact build is [Actions run 34079729758](https://github.com/yan-h/harmonigraph/actions/runs/34079729758).
Raw local logs and the alternating-process runner are in `/tmp/harmonigraph-precompiled-blur-results/`;
the durable investigation is [issue #699](https://github.com/yan-h/harmonigraph/issues/699).

## Build the libraries

The export test uses Naga and does not need a GPU or Apple's compiler:

```sh
export HARMONIGRAPH_METAL_PROBE_DIR=/tmp/harmonigraph-precompiled-blur
cargo test --release -p harmonigraph-render --lib export_shadow_blur_metal -- --ignored --nocapture
python3 tools/metal-precompile/compile.py "$HARMONIGRAPH_METAL_PROBE_DIR"
```

The last command requires `xcrun metal` and `xcrun metallib`.
The Metal precompile probe workflow performs these steps on a macOS runner and uploads the small `precompiled-shadow-blur` artifact,
so a developer with Command Line Tools alone can download it and run the local measurement.
No Apple toolchain binaries are included in the artifact.

Verify downloaded artifacts before loading their compiled shader code:

```sh
python3 tools/metal-precompile/compile.py "$HARMONIGRAPH_METAL_PROBE_DIR" --verify
cargo test --release -p harmonigraph-render --lib precompiled_shadow_blur_probe -- --ignored --nocapture
```

The probe requires a real Metal adapter and fails if none exists.
It rejects exported source or entry names that differ from what the current Naga/shader/layout generates.
Parity compares two complete lattice frames at different Gaussian shadow widths,
then requires that replacing the blur with an erased result visibly changes each fixture.
This prevents a passing comparison of two pictures that never exercised the pipeline.

## Measure cold construction

Compile the test binary once and run it directly in a fresh process for each sample,
with `HARMONIGRAPH_METAL_PROBE_MODE=wgsl` or `metallib`.
Both modes request the same device features and build just the selected pipeline after device creation.
The printed `total_ms` includes shader-module loading and pipeline creation;
the metallib mode also prints the library-loading part.
Generated-source validation occurs before the timed section.

Use the process-local Metal disk-cache denial profile documented in [issue #699](https://github.com/yan-h/harmonigraph/issues/699),
and alternate mode order across trials.
Run the executable under `sandbox-exec`, not Cargo or the compiler.
Do not delete or rename global cache files.

These measurements concern one pipeline,
not whole-editor startup or the original long-idle Bitwig symptom.
The two Metal IR libraries can still require GPU-specific pipeline compilation at runtime.

## Scope and maintenance cost

The explicit resource and vertex-buffer mappings in the export test match the pinned wgpu-hal 29 Metal implementation.
They are not a stable public wgpu layout contract.
The test retains generated vertex bounds checks and uses the production `ShadowBox` layout,
but passthrough does not provide the normal shader reflection metadata.
In particular, runtime storage-array size bindings need separate investigation before applying this to other pipelines.

The experiment keeps this mapping and an equivalent pipeline descriptor in test code;
shipping precompilation would need a maintained source of that information and coverage for supported devices,
shader changes, layout changes and hot reload.
It introduces no persistent runtime cache or background compilation threads.
