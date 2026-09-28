# Stars next-experiment evidence

This is an evidence-only draft branch for #1142, not a plugin change or a proposal to ship these files.
Read [REPORT.md](REPORT.md) for the conclusions, exact baselines and limitations.
The production tree is unchanged outside this evidence directory.

## Reproduce

Start in an owner-managed worktree at commit `7711b98d2c28c737b5a9c44dd298eb3d96efe589`.
Copy this evidence directory outside the checkout before changing refs.
Set `bundle` below to that absolute copy path.
The scratch destination must be free or belong to this reproduction.

```sh
bundle=/absolute/path/to/next-experiments-2026-09-27
git apply "$bundle/research.patch"
mkdir -p /private/tmp/stars-next
cp "$bundle"/shaders/*.wgsl /private/tmp/stars-next/
cp "$bundle"/{run.py,images.py,analyze.py} /private/tmp/stars-next/
cargo test --release -p harmonigraph-render --lib --no-run
HARMONIGRAPH_SHADER_ASSETS=source HARMONIGRAPH_REQUIRE_GPU=1 \
  cargo test --release -p harmonigraph-render star_blur::probe -- --nocapture --test-threads=1
RESEARCH_WORKTREE="$PWD" PROBE_INPUT=synthetic \
  python3 /private/tmp/stars-next/run.py complete-reproduction
RESEARCH_WORKTREE="$PWD" PROBE_INPUT=synthetic \
  RESEARCH_CASES=p3-a,p3-b,p3-split1,p3-split3,p3-nosplit,p3-split2 \
  python3 /private/tmp/stars-next/run.py partition-reproduction
RESEARCH_WORKTREE="$PWD" PROBE_INPUT=synthetic PROBE_JITTER=1 \
  RESEARCH_CASES=p3-a,p3-b,full-a,blur-a,blur-b,blur-fused \
  python3 /private/tmp/stars-next/run.py fusion-reproduction
```

Python analysis requires NumPy; contact-sheet generation additionally requires Pillow.
The recorded fixture is not included: `take-levels.u8`, `flat-levels.u8`, and `palette.rgba` are expected in `/private/tmp/stars-full-halo-compare`.
Synthetic timing and the independent convolution probe need none of those files.
Do not run multiple GPU probes, builds, captures or video encodes concurrently with timing.
Each run creates a new output directory and refuses to overwrite an existing one.

The cumulative patch includes the test-only blur port.
For exact first-stage harness provenance, `stage12` preserves the original small hook patch and both test modules; copy those modules into `crates/harmonigraph-render/src/spectrogram/tests` after applying that hook patch instead of the cumulative patch.
The main-shader substitutions use external WGSL files, whose hashes are recorded in each run manifest.
The standalone blur shader is embedded in the cumulative patch.
The native production shaders and supported pipeline catalog are unchanged.
No Metal corpus regeneration or release plugin build is needed for this final evidence-only diff.

## Contents

- `timings`: raw samples, logs, input settings, binary/source hashes and descriptive block-bootstrap analysis.
- `images`: capture settings, byte-error metrics and selected contact sheets; raw private captures are omitted.
- `research.patch`: cumulative scratch harness and fused-filter source against the pinned baseline.
- `shaders`: exact source substitutions loaded by the harness.
- `stage12`: original complete/partition harness before the test-only blur port.
- `blur-probes.log`: independent CPU-reference seed and fused-convolution checks.

The old blur construction comes from the gathered-blur prototype plus full-filter patch in immutable snapshot `03ecc492f2220a96c0949c88fcdc8180316b8ae5`.
The complete-response helper comes from immutable snapshot `a5ed674ea18427001ea9029fb9f8c17f3827c116`, adapted to the merged grouped targets.
The private fixtures are reused, not regenerated.
