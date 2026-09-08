# Metal shader assets

The renderer loads embedded Metal libraries through the normal wgpu backend compilation boundary.
WGSL and the existing Rust pipeline constructors remain the authoring sources.
The [feasibility record](generated-shader-assets.md) explains why this boundary preserves metadata while avoiding substantial cold source-compilation work.

## Ownership and ordinary builds

`harmonigraph-metal-assets` contains verified immutable source/options/library triples and has no graphics-stack dependency.
Its build script checks the complete file set and SHA-256 hashes, then embeds the inputs and libraries.
Normal `cargo build` needs neither Apple's shader compiler nor a download step.
Both plugin and offline binaries contain their own bytes, so the existing executable-inode installation and old mapped-plugin lifetimes apply without sidecar installation.

The renderer installs one immutable provider before any device request.
The [small backend patch](../PATCHES.md) owns only library loading; normal Naga specialization, bindings, bounds checks, reflection and GPU-object lifetimes remain in wgpu.
The existing asynchronous startup worker and retained-device reopen contracts are unchanged.
No new worker or global native-object cache is introduced.

Exact generated source, normalized compiler options, schema and backend version determine lookup identity.
The deterministic hash only chooses a bucket; full input equality is mandatory.
Scene state and device pointers do not belong in that identity.
Compiler provenance and deployment target are recorded in the manifest.
The current producer supports Metal 3.2, macOS 15+, fast math and preserved invariance.
The producer derives compiler flags from the recorded options and verifies that every manifest agrees;
changing that mapping requires regenerating and validating the corpus.
Other inputs or native-library load failures use ordinary source compilation and report the fallback.
An Apple Silicon corpus does not promise complete Intel internal-shader coverage.

## Generation and validation

The feature-gated `production_metal_asset_catalog` calls the actual lattice, text, roll, spectrogram, glow, dot-shadow, shared-shadow and egui constructors.
It exercises default/offline and native/timestamp device profiles with BGRA8/RGBA8 linear and sRGB targets.
Pipeline state stays defined at its existing owner.
Synthetic reference shaders from other tests are not shipping assets.

Check the committed corpus on a real Metal adapter:

```sh
python3 tools/shader-assets.py check
```

This requires zero source fallback in the constructor catalog and runs real-path renderer and offline goldens in strict mode.
Ordinary reference tests retain normal fallback, so their test-only shaders do not pollute the shipping corpus.
The regular workspace gates remain required too.
Generation also builds a separate temporary storage-array diagnostic corpus and verifies strict rejection/source fallback for missing, mismatched and invalid libraries.
Those diagnostic libraries are never imported into the shipping corpus.
The PR workflow runs these controls after checking committed coverage; they can also be run with `python3 tools/shader-assets.py controls` when Apple's compiler is available.
Generation and controls use `HARMONIGRAPH_METAL_ASSET_BUILD_DIR` to embed temporary corpora into their test binaries at build time.
They never replace the tracked corpus, even during deliberate invalid-library controls, so an interrupted run cannot leave those controls in an ordinary build.
Cargo tracks this build input and rebuilds with the checked-in corpus when the variable is absent.
Do not run other Cargo builds in the same checkout concurrently with validation: a second build could replace a test executable between its build and execution.

With Apple's Metal toolchain available, generate into a fresh directory:

```sh
python3 tools/shader-assets.py generate target/new-metal-assets
python3 tools/shader-assets.py import target/new-metal-assets
```

Generation exports exact backend inputs, compiles and verifies them, embeds the candidate corpus in validation binaries, then checks it through the normal production path.
Only explicit import replaces the checked-in assets.
Inspect and commit the changed source/options/manifest together with the binary libraries.
When local Command Line Tools lack `metal`/`metallib`, run the Metal shader assets workflow with `regenerate=true` on the intended branch, then download and import its validated `production-metal-assets` artifact.
The ordinary PR workflow checks the committed corpus without regenerating it, so generation cannot conceal stale coverage.
Its path filter covers assets, renderer code, graphics setup and dependencies;
ordinary tuning implementation edits do not schedule a third macOS job.
New pushes cancel superseded asset-validation runs.

## Diagnostics and measurements

`HARMONIGRAPH_SHADER_ASSETS` selects `embedded` (default), `source` (comparison baseline) or `strict` (missing/invalid artifacts fail pipeline creation).
Mode is fixed before the first device and shared by that linked image; use fresh processes when comparing modes.
The tooling feature additionally permits `export` with `HARMONIGRAPH_SHADER_EXPORT_DIR`.
There is no runtime sidecar override.
`shader_assets::statistics()` reports actual backend library loads, source calls, load failures and rejected requests.

Native editor timing, package checks and controlled GPU comparisons must accompany the production handoff.
The optional native probe uses the production graphics configuration, loading UI and frame pacing in three parented editors with one retained device:

```sh
cargo build --release -p harmonigraph-plugin --example editor-startup --features startup-probe
HARMONIGRAPH_SHADER_ASSETS=strict target/release/examples/editor-startup
HARMONIGRAPH_SHADER_ASSETS=source target/release/examples/editor-startup
```

It activates its own temporary host window so Metal can present, closes each editor from the host callback, joins initialization and returns normally.
It asserts all three editor states were dropped and exactly one device was created.
First/full-frame times are observed on the next UI update after a paint callback; they include up to one subsequent update interval.
Headless cache-blocked measurements do not establish natural long-idle Bitwig behavior or physical display latency.
The installed DAW slot is selected by the user through `load-plugin.sh`; generation and validation never swap it.

## Measured production-path results

The [initial generator](https://github.com/yan-h/harmonigraph/actions/runs/34175684884) produced 65 libraries, totaling 575,815 library bytes and 1,090,702 bytes including source/options/manifest.
Strict catalog replay made 820 library loads with zero source calls.
All 14 renderer goldens and five offline goldens passed without re-baselining.
The [expanded control run](https://github.com/yan-h/harmonigraph/actions/runs/34176508889) passed compiled storage-array binding checks at two lengths and two offsets.
Missing, mismatched and invalid library controls each failed strictly and reproduced the expected values through exactly one source fallback in normal mode.

Five alternating fresh-process pairs per condition on the Apple M1 Pro measured the current asynchronous native editor path:

| Median observed stage | Cache blocked, source | Cache blocked, assets | Cache available, source | Cache available, assets |
| --- | ---: | ---: | ---: | ---: |
| First frame, first opening | 305.7 ms | 147.9 ms | 79.2 ms | 75.6 ms |
| Full ready, first opening | 4,770.8 ms | 766.0 ms | 95.8 ms | 95.2 ms |
| Full ready, second opening | 453.7 ms | 59.6 ms | 24.4 ms | 21.9 ms |
| Full ready, third opening | 448.2 ms | 59.2 ms | 21.4 ms | 21.1 ms |

Each process completed three real window lifetimes with one retained device and three dropped window states.
Strict processes reported 106 library loads with zero source calls, load failures or rejected requests.
Cache-blocked first full readiness improved about 84% in this series; warm readiness was essentially unchanged.
The cold reopen cost includes fresh egui/window work even though lattice pipelines and the device are retained.
See the [durable evidence](evidence/production-metal-assets/README.md) for every sample, binary provenance and the process-local cache restriction.
Actual Bitwig active-audio contention and natural long-idle behavior remain host checks for the loadable draft.

The existing GPU probes ran eight alternating source/strict pairs after an unmeasured warmup pair, with 240 frames per workload and actual backend-path counters checked.
Median GPU times across the per-run medians were:

| Workload | Source | Assets |
| --- | ---: | ---: |
| Gaussian, live view | 0.787 ms | 0.762 ms |
| Gaussian, top of shadow bar | 0.579 ms | 0.585 ms |
| Distance, live view | 0.548 ms | 0.546 ms |
| Distance, top of shadow bar | 0.513 ms | 0.528 ms |
| 225 audio rings, no MIDI glow | 0.719 ms | 0.620 ms |
| 225 audio rings, one MIDI glow | 0.724 ms | 0.740 ms |
| Repeated Distance live reference | 0.549 ms | 0.534 ms |
| Repeated top-of-bar reference | 0.516 ms | 0.516 ms |

These samples show no consistent GPU slowdown: paired differences change sign, and the small median shifts are within the observed run-to-run spread.
The no-glow audio-ring workload is particularly noisy; its lower asset median is not a reliable speedup claim.
The measurement covers the prepare encoder, including scene/shadow/bloom work, and excludes the final egui composite.
It is representative evidence on this M1 Pro, not a guarantee for every GPU or workload.
No authoring shader, precision or optimization setting was downgraded.
