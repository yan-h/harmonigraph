# Generated Metal shader assets

This is the feasibility investigation for [#699](https://github.com/yan-h/harmonigraph/issues/699).
The production plugin and offline renderer still use their normal WGSL path.
The tooling in `tools/metal-precompile/` modifies an isolated copy of pinned wgpu-hal 29.0.4 for an opt-in experiment;
no backend fork or generated libraries are added to the production dependency graph.

## Recommendation and scope

Make compiled Metal libraries automatically generated build assets while keeping the existing renderer.
The isolated experiment establishes that this is feasible for the exercised renderer paths, with a substantial cache-blocked construction improvement.
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

The source, strict asset and controlled fallback paths all produced these results:

| Buffer offset | Array length | Output |
| --- | ---: | --- |
| 0 | 4 | `[4, 11, 44, 44]` |
| 0 | 7 | `[7, 11, 77, 77]` |
| 256 | 4 | `[4, 11, 44, 44]` |
| 256 | 7 | `[7, 11, 77, 77]` |

Production image coverage and fallback controls passed in the workflow described below.

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

The [macOS CI experiment](https://github.com/yan-h/harmonigraph/actions/runs/34172958788) passed at probe commit `06d3f0a8`.
The ordinary Full CI and security checks also passed at that commit.
The subsequent documentation/evidence commit does not change the probe implementation.

| Strict asset replay | Passed | Ignored | Library loads | Source compilations |
| --- | ---: | ---: | ---: | ---: |
| Storage-binding diagnostic | 1 | 0 | 3 | 0 |
| Renderer suite | 243 | 8 | 11,830 | 0 |
| Offline goldens | 5 | 2 | 126 | 0 |

The same suites passed in source/export mode, and no golden was changed.
Both missing-library and mismatched-source controls failed in strict mode;
fallback mode reproduced all four diagnostic outputs with exactly one source compilation.
Normal-path reflection remained present for real shadow vertex/fragment programs and the glow-gather program with two runtime-storage bindings.
This goes beyond replaying a texture-only blur or a synthetic shader.

The exported test corpus contains 70 libraries totaling 676,057 bytes, about 660 KiB before any executable packaging overhead.
It includes test-only programs and is not a complete shipping manifest.
The compiler was Apple Metal `32023.883` with the options documented above.
The downloaded corpus passed SHA-256/file-set verification and then the diagnostic and all 243 renderer tests on the local Apple M1 Pro.
That is evidence of CI-to-local library reuse for this configuration, not an OS/GPU compatibility matrix.
The durable [validation record](evidence/generated-shader-assets/validation.txt), [CI counters](evidence/generated-shader-assets/ci-results.json) and [artifact manifest](evidence/generated-shader-assets/asset-manifest.json) preserve the results independently of the expiring CI artifact.

### Startup measurements

The same local release test binary ran `timing_editor_pipeline_startup` with an empty artifact directory/source fallback versus strict downloaded assets.
Three fresh processes per mode and cache condition used alternating source/asset, asset/source, source/asset order.
The table reports the median of the first opening in each fresh process;
all three openings per process are retained in the [raw timings](evidence/generated-shader-assets/startup-timings.json).

| Measured stage | Cache blocked, source | Cache blocked, assets | Cache available, source | Cache available, assets |
| --- | ---: | ---: | ---: | ---: |
| Device setup | 121.4 ms | 88.9 ms | 22.6 ms | 19.4 ms |
| Lattice resource construction | 4,565.6 ms | 684.6 ms | 26.6 ms | 28.6 ms |

Cache-blocked lattice construction fell about 85%, with source samples of 7,185.1 / 4,041.1 / 4,565.6 ms and asset samples of 646.3 / 684.6 / 737.3 ms.
The high first source sample is retained.
Warm-cache ranges overlap, so these samples establish no reliable warm improvement.
Every asset process reported 156 hits and zero source compilations;
every source process reported the reverse.
Both modes include the hook's identity work and logging;
asset mode additionally reads and compares source/options sidecars and loads library files.
The separate full-corpus SHA-256 verification happens before timing.
The local backend copy uses the same asset hook as CI, without the later reflection logging.

These are headless `LatticeResources::new` measurements with BGRA8 targets, including normal translation/reflection and final pipeline creation.
They exclude the editor window, asynchronous worker scheduling, egui setup and first-visible/full-ready presentation.
The process-local sandbox denied reads/writes to the known Metal cache directory without deleting caches;
it does not prove all compiler-service caches were cold or reproduce natural long-idle Bitwig behavior.
Do not extrapolate this percentage to whole-editor startup or add it to historical #699 timing series.

### Steady-state GPU sampling

The existing `a_frame_of_names_at_each_kernel_costs_this_much` probe ran three alternating fresh-process pairs on the same binary, with caches available.
Each workload used ten warmup frames followed by 120 measured frames at 768 × 768, with 30 names and 355 lit nodes.
The table gives the median of the three per-run GPU medians.

| Workload | Source | Assets |
| --- | ---: | ---: |
| Gaussian, live view | 0.764 ms | 0.802 ms |
| Gaussian, top of shadow bar | 0.635 ms | 0.641 ms |
| Distance, live view | 0.537 ms | 0.575 ms |
| Distance, top of shadow bar | 0.649 ms | 0.564 ms |

GPU timestamps cover the prepare encoder, including scene, label/shadow and bloom work;
they exclude the final egui composite.
The [raw results](evidence/generated-shader-assets/gpu-timings.json) retain every run and its p10/p90 spread.
The spread and first-source-run disturbance are large enough that this does not establish unchanged GPU performance.
Three of four medians are slightly higher with assets, while one is lower;
controlled broader GPU measurements remain a release gate.
No shader optimization downgrade or source rewrite was required to obtain the startup benefit.

### Feasibility decision

The critical library boundary works: existing constructors and backend metadata can use generated assets, pass the exercised pixel tests, handle controlled misses and substantially reduce cache-blocked construction time.
This supports implementing the asset architecture as the preferred next investment.
A strict suite pass covers exercised inputs, not every pane/format/variant, and does not establish current native editor readiness, unload behavior, production installation or universal GPU parity.
The production gates below remain necessary before enabling it in a release.

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

The library boundary is proved for the exercised inputs.
A production implementation should proceed through these gates:

1. Define the narrow provider boundary and upgrade contract against the pinned backend, preferably upstreamable.
   It owns artifact lookup only; Naga and backend reflection retain their existing owners.
2. Enumerate real constructors and supported variants, including egui, internal shaders, native/offline formats and device-dependent generation options.
   Strict release validation must expose missing assets instead of silently warming a source cache.
3. Automate export, compilation, verification and embedding for both binaries, with explicit toolchain/target provenance and regeneration on shader or backend changes.
   Test development fallback and package/load behavior, including an older plugin still mapped by the host.
4. Measure native first-visible/full-ready cold and warm startup against the current asynchronous baseline, retained-device reopen, natural long-idle Bitwig behavior, active-audio contention, unload and memory.
   Require broader controlled GPU timings and unchanged exercised pixels before release.

Stop and reassess if this needs manual per-shader binding tables or an invasive permanent backend fork.
The prototype's small replacement point is encouraging, but it is not evidence that future backend upgrades will be free.

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
