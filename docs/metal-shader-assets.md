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

With Apple's Metal toolchain available, generate into a fresh directory:

```sh
python3 tools/shader-assets.py generate target/new-metal-assets
python3 tools/shader-assets.py import target/new-metal-assets
```

Generation exports exact backend inputs, compiles and verifies them, temporarily embeds the candidate corpus, then checks it through the normal production path.
It restores the caller's corpus even on validation failure.
Only explicit import replaces the checked-in assets.
Inspect and commit the changed source/options/manifest together with the binary libraries.
When local Command Line Tools lack `metal`/`metallib`, run the Metal shader assets workflow with `regenerate=true` on the intended branch, then download and import its validated `production-metal-assets` artifact.
The ordinary PR workflow checks the committed corpus without regenerating it, so generation cannot conceal stale coverage.

## Diagnostics and measurements

`HARMONIGRAPH_SHADER_ASSETS` selects `embedded` (default), `source` (comparison baseline) or `strict` (missing/invalid artifacts fail pipeline creation).
Mode is fixed before the first device and shared by that linked image; use fresh processes when comparing modes.
The tooling feature additionally permits `export` with `HARMONIGRAPH_SHADER_EXPORT_DIR`.
There is no runtime sidecar override.
`shader_assets::statistics()` reports actual backend library loads, source calls, load failures and rejected requests.

Native editor timing, package checks and controlled GPU comparisons must accompany the production handoff.
Headless cache-blocked measurements do not establish natural long-idle Bitwig behavior or physical display latency.
The installed DAW slot is selected by the user through `load-plugin.sh`; generation and validation never swap it.
