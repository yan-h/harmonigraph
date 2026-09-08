# Generated asset experiment evidence

See the [design and interpretation](../../generated-shader-assets.md) before comparing these numbers.
This directory preserves measured results from probe commit `06d3f0a8`, not a shipping shader bundle.

- `ci-results.json`: source/export, strict replay and deliberate failure/fallback counters.
- `asset-manifest.json`: Apple compiler provenance, flags and SHA-256 for all 70 source/options/library sets.
- `validation.txt`: original test outcomes, selected reflection records, local replay counts, log hashes and timing summaries.
- `startup-timings.json`: every opening in all twelve fresh-process startup runs.
- `gpu-timings.json`: every workload's median and p10/p90 from all six fresh-process GPU runs.

The [CI run](https://github.com/yan-h/harmonigraph/actions/runs/34172958788) uploaded full logs and compiled libraries as `generated-metal-assets`, with seven-day retention.
After expiration, regenerate them from probe commit `06d3f0a8` with `tools/metal-precompile/exercise.py` as described in the design document.
The compiler version may change, so compare the new manifest rather than assuming byte-identical compiler output.

## Reconstructing the local measurements

These commands apply to probe commit `06d3f0a8`.
The named `exercise.py` and `backend.py` tools were removed from the current tree;
use the [production workflow](../../metal-shader-assets.md) for current code.

Build the render test binary with the isolated backend configuration created by `backend.py prepare`, using the build procedure in `exercise.py` so the temporary path patch does not persist in `Cargo.lock`.
Verify the downloaded or regenerated asset directory with `backend.py verify`.
Keep one binary for both modes.
The recorded local binary hash and platform are in `validation.txt`.

Set `HARMONIGRAPH_REQUIRE_GPU=1` and `WGPU_BACKEND=metal`.
For source mode, set `HARMONIGRAPH_METAL_ASSETS=fallback` and `HARMONIGRAPH_METAL_ASSET_DIR` to an empty directory.
For asset mode, use `strict` and the verified asset directory.
Launch the test binary separately for each sample, with these arguments:

```text
timing_editor_pipeline_startup --ignored --nocapture --test-threads=1
a_frame_of_names_at_each_kernel_costs_this_much --ignored --nocapture --test-threads=1
```

Use source/asset, asset/source, source/asset order for three pairs in each condition.
The startup test creates three fresh renderers per process;
`opening 0` supplies the reported first-opening median, while the other openings remain separate in the raw data.
Run the GPU probe with `PROBE_FRAMES=120` and caches available.
Require successful exit, actual reported timing samples, positive asset hits with zero source compilations in strict mode, and positive source compilations with zero hits in source mode.
The recorded GPU runner checked all four workload results and those hit/source conditions before accepting each process.

For the recorded cache-blocked startup runs only, prefix the command with `/usr/bin/sandbox-exec -f PROFILE`.
The exact profile was:

```scheme
(version 1)
(allow default)
(deny file-read* file-write*
  (subpath "/private/var/folders/3f/bjndnzhs0t50k9qpl1hpm_l40000gn/C/com.apple.metal"))
```

That directory is machine-specific;
resolve the actual canonical Metal cache directory before reproducing on another machine.
This restricts the tested process, does not delete caches, and does not guarantee every compiler-service cache is cold.
Neither these commands nor the isolated backend experiment install a plugin or operate Bitwig.
