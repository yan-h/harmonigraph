# Generated Metal shader assets

This is the feasibility investigation for [#699](https://github.com/yan-h/harmonigraph/issues/699).
The production plugin and offline renderer still use their normal WGSL path.
The tooling in `tools/metal-precompile/` modifies an isolated copy of pinned wgpu-hal 29.0.4 for an opt-in experiment;
no backend fork or generated libraries are added to the production dependency graph.

## Recommendation and scope

Make compiled Metal libraries automatically generated build assets while keeping the existing renderer.
Retain one authoring copy of WGSL and the production Rust pipeline constructors.
Let the pinned backend continue owning native bindings, vertex pulling, specialization, bounds checks and reflection.
The initial integration should replace only the expensive source-library compilation call with an exact artifact lookup and library load.

The historical #699 native profile spent roughly 3.9 seconds compiling Metal source and 0.61 seconds creating final pipeline states.
Naga specialization and MSL generation took about 18 ms.
Those are historical synchronous measurements, not current asynchronous editor timings or a predicted saving.
Retaining the small translation stage is a reasonable way to preserve backend metadata while addressing the larger compilation stage.

High upfront effort can buy a simpler ongoing workflow:
edit WGSL or a pipeline definition once, then regenerate assets automatically.
The recurring costs become the Apple toolchain, artifact coverage, packaging and backend-upgrade checks.
Hand-maintained Metal slot maps for every shader would defeat that purpose.

## Ownership

| Owner | Responsibility |
| --- | --- |
| WGSL sources | Shader behavior. |
| Production Rust constructors | Entry points, layouts, vertex formats, targets, blend/depth state and variants. |
| Pinned wgpu/Naga backend | Native binding allocation, specialization, vertex pulling, generated source and reflection. |
| Artifact builder | Exercise the same constructors, compile generated inputs and verify source/options/binary identity. |
| Backend artifact provider | Load matching immutable bytes on the actual device; preserve source fallback. |

A future catalog should enumerate existing constructors and supported variants.
It must not reproduce the backend's private binding allocation.
Coverage includes the lattice, text, spectrogram, roll, glow and spiral shadow paths, plus egui and device-internal shaders.
Both native BGRA8 and offline RGBA8 configurations matter.
One default scene is not evidence of complete coverage.

## The critical boundary

The isolated hook is inside `wgpu-hal::metal::Device::load_shader`, immediately around `newLibraryWithSource`.
Ordinary Naga processing occurs before the lookup;
ordinary entry-point lookup and `CompiledShader` reflection occur after it.
The library comes from either source compilation or bytes loaded through the backend's existing metallib loader.
Nothing changes the resulting storage-size bindings, workgroup-memory sizes or immutable-buffer mask.

This differs materially from the earlier public passthrough probe:
passthrough supplies empty storage-size/workgroup metadata and a zero immutable-buffer mask in the pinned backend.
Real text/lattice/spectrogram shaders use runtime storage arrays.
Their bounds checks and `arrayLength` require the lengths uploaded using that metadata.
[Upstream Metal limitation](https://github.com/gfx-rs/wgpu/issues/8330).

The experiment also logs reflection after the common path, and requires strict replay to report asset hits with no source fallback.
The diagnostic checks one allocation bound at lengths of four and seven array elements, at offsets zero and 256 bytes.
Its outputs depend on `arrayLength`, the first/last element and an out-of-range read clamped by the pinned backend's Restrict policy.
The initial two-element range was rejected by wgpu validation because the struct's vec4 prefix makes its minimum binding size 32 bytes;
the final fixture uses valid ranges that actually reach the shader.

The ordinary source-path results are:

| Buffer offset | Array length | Output |
| --- | ---: | --- |
| 0 | 4 | `[4, 11, 44, 44]` |
| 0 | 7 | `[7, 11, 77, 77]` |
| 256 | 4 | `[4, 11, 44, 44]` |
| 256 | 7 | `[7, 11, 77, 77]` |

Compiled replay, production image coverage and fallback controls are executed by the workflow described below.
Their measured results belong in the results section before treating the design as established.

## Reproduction

The local machine has Command Line Tools but no `metal` or `metallib` executable.
The existing macOS CI job can obtain Apple's Metal toolchain, as the earlier #704 probe already does.
No local toolchain installation is required to inspect exported inputs or run downloaded libraries.

From the repository root, use a fresh experiment directory:

```sh
python3 tools/metal-precompile/backend.py prepare /tmp/generated-metal-assets
python3 tools/metal-precompile/exercise.py /tmp/generated-metal-assets
```

The exercise requires a real Metal adapter and the Apple offline compiler.
It builds test binaries with a temporary Cargo path patch, restoring the original `Cargo.lock` after each build.
The registry sources and normal dependency configuration remain unchanged.
The copied backend is isolated from release plugin builds.

The exercise runs the binding diagnostic, the ordinary renderer suite and offline goldens in source/export mode.
It compiles and verifies all exported inputs, then repeats those tests in strict asset mode.
Finally it withholds the diagnostic's library and mismatches its source sidecar:
strict mode must fail and fallback mode must produce the expected numeric results through exactly one source compilation.
No golden is re-baselined.
The workflow uploads the libraries, raw logs and `results.json`.

The backend supports three explicit probe modes through `HARMONIGRAPH_METAL_ASSETS`:
`export`, `strict` and `fallback`.
`HARMONIGRAPH_METAL_ASSET_DIR` supplies the experiment's artifact directory.
With the mode unset, even the copied backend takes the unchanged source path.
These environment variables are experimental tooling, not a proposed production API.

The file bucket uses deterministic FNV64 over source and options, with mandatory exact source/options comparison before loading.
That comparison prevents a hash collision from selecting mismatched code.
The builder separately records SHA-256 for every exported source, option file and library, and verifies the complete file set before replay.
A production bundle should use a defined artifact format and verified immutable embedded bytes;
this file-based probe does not claim to implement that packaging boundary.

The supported experiment configuration is Metal 3.2 with a macOS 15 minimum target, fast math and preserved invariance.
It records the resolved modern math mode and floating-point function policy as well as the legacy fast-math value.
The builder refuses other settings rather than silently compiling different options.
Compiler provenance is recorded in `manifest.json`.

## Results and limits

The ordinary source diagnostic passed on an Apple M1 Pro, including both lengths and both offsets.
Asset replay and image results are pending the branch's CI experiment.

The probe can establish metadata preservation, actual image parity, tested-input coverage and controlled fallback behavior.
It does not establish a current native first-visible/full-ready editor improvement, natural long-idle Bitwig behavior, unchanged steady-state GPU timing, all supported configurations, or production package installation.
A strict pass covers the exercised tests, not every possible future shader or pane configuration.
Source-versus-asset timing must use the same instrumented binary and explicitly account for logging and file verification.

## Production packaging

Prefer embedding generated manifest/library bytes in a shared Rust asset crate initially.
The plugin loader swaps a signed executable inode, while the offline renderer is installed as a standalone executable.
Embedded assets follow those existing identities and remain valid for an older plugin image still mapped by a host process.
Sidecar files would introduce coordinated resource installation and retention for old loaded builds.
Embedding duplicates asset bytes across plugin and offline binaries;
measure the complete artifact size before deciding whether that cost warrants a more elaborate package format.

The provider must be configured before device initialization to cover internal shaders, not only after lattice construction begins.
Its key must cover exact generated source and relevant compiler/backend/target inputs.
Scene animation and other unrelated UI state do not belong in that key.
Native Metal library and pipeline objects remain tied to their actual device.
Unexpected release-coverage misses should fail validation visibly, while development and unsupported targets retain source fallback.

## Subsequent decisions

After the library boundary is proved, a production implementation still needs constructor enumeration, embedded assets, build integration and native cold/warm measurements.
Preserve the existing retained-device/reopen and asynchronous-loading contracts.
Measure active-audio contention, unload behavior, retained memory and steady-state GPU time.

GPU binary archives are a separate optional layer targeting final pipeline compilation.
Apple can harvest real pipeline descriptors and generate GPU-specific archives from them.
Archives add package/target coverage and can be invalidated by OS changes, so keep Metal IR fallback.
They are not necessary to test the dominant source-compilation opportunity.
[Apple archive workflow](https://developer.apple.com/documentation/metal/creating-binary-archives-from-device-built-pipeline-state-objects).

Prefer a small upstream-compatible integration over a broad permanent fork.
Wgpu's [precompiled-shader tracking issue](https://github.com/gfx-rs/wgpu/issues/9052) identifies the same hidden-logic and validation problems;
the linked `wgpu-shaders` extraction PR #9080 was closed without merging when checked.
Upstream support is a direction, not a dependency on a promised delivery date.

If keeping wgpu instead demands an invasive permanent fork, compare a complete macOS-only Metal renderer seriously.
It can retain egui UI logic but must own the painter, resource lifetimes, synchronization, presentation and readback.
Keep one renderer shared by the editor, standalone and offline paths.
A persistent helper process relocates the first cold cost and adds service/IPC/version lifetimes;
reducing shader variants or merging passes may add runtime branches, attachment traffic or changed pixels.
Neither is the preferred way to obtain the library-precompilation benefit.
