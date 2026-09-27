# Temporary production image parity capture

This patch is scratch evidence only.
Apply after the scene/UI and production renderer patches, capture, then reverse it before committing.
It adds one ignored test module and its module declaration.
It does not use research source overrides, ACTIVE state, pipeline substitutions, group-map constants, or render-path forcing.
No build or GPU run was performed while preparing it.

## Fixture identity

- Actual settings: `harmonigraph_scene::StarHaloProfile::{Uniform,P2,P3}` plus existing `star_halo_resolution`.
- Half = Uniform/0.5; Full = Uniform/1.0; P2 and P3 = their actual profile with scalar1.0.
- Appearance values are explicitly pinned to the defaults at frozen base `fd7c8f9f9fccb2a55f79f8ee87c25e2d5afd8d40`.
  Inert material settings retain their default; every Stars-relevant setting is explicit.
- Same private 1024-bin ×960-slab take/flat levels and4096-entry palette.
  `fixture-provenance.json` records their SHA256 hashes.
- Same zero-origin960×540-point pane, pitch range, read mapping, light settings and20-second visible time span.
- Fresh callback resources for every case/input;60 frames at times1 through2.966666666666667, then frame60 at time3.
  Memory stays enabled exactly as in the accepted research capture.
- PPP2 yields1920×1080; PPP4 yields3840×2160; PPP1.001 yields961×541 via f32 multiplication and ceil, matching the original frame helper.
- Same production prepare/paint/readback helper behavior, output RGBA8Unorm.
  The ordinary headless device requests production limits, without the now-unneeded research SHADER_F16 feature.
- `make_patch.py` asserts inherited fixture helper bodies still match the frozen spectrogram source byte-for-byte.
  It also checks patch applicability without applying it.

The normal production route chooses allocation, runtime group mapping and split rendering.
Therefore this measures whether the production implementation reproduces the accepted pixels, not whether two research variants agree.
No mathematical reference image or alternate shader participates.

## Apply and capture (root agent only)

From the research worktree, after production integration:

```sh
/opt/homebrew/bin/python3 /private/tmp/stars-investigation/live-parity-proposal/make_patch.py
git apply /private/tmp/stars-investigation/live-parity-proposal/live-parity.patch
LIVE_PARITY_OUTPUT=/private/tmp/stars-investigation/images-live-1080 LIVE_PARITY_PPP=2 cargo test --release -p harmonigraph-render spectrogram::tests::live_parity::stars_live_production_images -- --ignored --exact --nocapture --test-threads=1
/opt/homebrew/bin/python3 /private/tmp/stars-investigation/live-parity-proposal/compare.py /private/tmp/stars-investigation/images-live-1080
```

Optional matching captures use `LIVE_PARITY_PPP=4` or `1.001` and distinct output directories.
The comparator chooses `images-final-4k` or `images-grouped-odd` automatically from the manifest.
It can also accept `--reference /absolute/reference/directory`.
Keep the existing reference manifest with any copied raw images.

The test writes its manifest only after all eight output images finish.
Use a fresh output directory for each run; do not interpret an old manifest alongside partially overwritten images.
Preserve the patch, source, provenance hashes, production commit/diff and test log together with outputs.

## Reading results

Half compares both `half-a` and the grouped uniform-half control when available.
Full compares both `full-a` and the grouped uniform-full control when available.
P2/P3 compare their accepted grouped factor variants.
Every pair reports exact-byte equality, changed bytes and pixels, maximum error, mean RGB error, pixels exceeding1LSB and alpha changes, plus source hashes.
There is no automatic tolerance-based acceptance.
Small nonzero differences require evaluation; do not label them exact parity.

Missing reference files are listed and cannot establish coverage.
At proposal creation `images-final-4k` contains only a manifest, and the odd-PPP reference has Full/P2 only.
Thus neither can currently prove all-four-case parity without finishing the frozen-reference captures.
The comparator exits1 for incomplete four-case coverage or any available pair mismatch, after saving all findings.

After evidence capture, from the same worktree:

```sh
git apply -R /private/tmp/stars-investigation/live-parity-proposal/live-parity.patch
```

If formatting or later edits changed the temporary hunks, remove only this module and its declaration manually.
Do not reverse the production scene, renderer, or UI patches.
