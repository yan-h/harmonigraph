# HarmoniGraph research evidence bundle

This scratch bundle preserves compact GPU timing evidence and the source needed to understand or reproduce the harness. It contains timing CSVs, per-size analyses, run manifests and logs; WGSL variants and proposed helper sources; a snapshot of the three renderer/test files hashed by the timing harness; and the curve optimizer's compact JSON, CSV and PNG outputs.

No RGBA frames, audio, or other private recorded fixtures are included. Runs whose manifest says `input: take` or whose log says `recorded take` need the private recorded fixture and its matching application state to reproduce. This bundle contains their measurements and metadata only. The optimizer source is included, but its raw RGBA inputs are omitted; its prior outputs are preserved for reference.

## Existing evidence

`timings/` contains compact artifacts copied as they existed during bundle preparation. The current analyzer was rerun on 55 CSVs with no failures; the generated inventory has 260 run-size-case records. `calibration/3840x2160` remains intentionally incomplete: its manifest lists the case set, but its CSV, log and analysis are absent. Other folders may represent separate experimental branches or profiles; compare each run's manifest and source hashes before combining results. Stored timing savings and interval values are percentages, and bootstrap intervals are descriptive rather than calibrated guarantees. The rerun audit preserves all 55 prior analyses and records six point-estimate, six interval, and three A/A changes across three analysis files.

## Reproduction

`run_batch.py` no longer assumes a fixed Codex worktree, scratch folder or executable. It accepts explicit paths, and writes new results below `<scratch>/timings/<label>`:

```sh
python3 scripts/run_batch.py \
  --worktree /path/to/harmonigraph-worktree \
  --scratch /path/to/research-scratch \
  --binary /path/to/harmonigraph_render_test_binary \
  --shader-root "$PWD/variants" \
  --frames 120 --warmup 60 --sizes 1920x1080,3840x2160 \
  --offset 0 --jitter 1 --input synthetic --profile default \
  example-run 'half-a,half-b,half-core,half-wide'
```

The `--binary` executable must already contain the `stars_research_timings` ignored test. Build it from the matching worktree with the repository's normal Rust workflow, then pass its executable path. Check that the `source-snapshot/` files match the worktree and that the manifest records the intended Git revision before running. Shader hashes are emitted separately by variant group. `--test-name` can select another test name when a matching harness exists.

The runner requires a working GPU and sets `HARMONIGRAPH_REQUIRE_GPU=1`; invoking it performs GPU measurements. The bundle was assembled without invoking the runner, analyzer, optimizer, builds or GPU operations. The current Rust harness still reads WGSL from `/private/tmp/stars-investigation/{shaders,separate-proposal,grouped-proposal,fusion-proposal}`. Copy the matching files from `harness-inputs/` into those exact directories on the test machine before running; the runner's `--shader-root` controls hash metadata, not the Rust harness's hard-coded read paths. The runner's `--scratch` selects the output root.

The `take` input also reads `/private/tmp/stars-full-halo-compare/take-levels.u8` and `palette.rgba`; those private fixtures are omitted. Arrange access to those exact files outside this bundle before using `--input take`. Do not treat synthetic-input timings as recorded-take timings.

`analyze_timings.py` analyzes existing CSVs when run directly. `curve_optimizer.py` needs complete `curve-p0`, `curve-p1`, ... still directories containing matched 1920×1080 RGBA scenes; those private/bulky inputs are not bundled. `image_report.py` needs take/flat RGBA files and the repository's lightweight PNG kit.

## Bundle contents and limits

- `timings/`: compact CSV, JSON manifest/analysis and text log files only.
- `variants/`: baseline/experimental WGSL and proposed Rust/Python helper, README and patch sources.
- `harness-inputs/`: WGSL files at the exact runtime names the current Rust test reads from its fixed `/private/tmp/stars-investigation` paths.
- `source-snapshot/`: exact renderer and test source files available in the source worktree at packaging time.
- `scripts/`: portable batch runner plus analysis/image/curve helper scripts.
- `curve-proposals.{json,csv,png}`: existing optimizer outputs; these are estimates/diagnostics, not a replacement for combined render validation.
- `images-*`, `profiles-*`, `atlas-take-analysis.json`, and `fusion-parity/`: compact numeric image metrics, manifests, parity CSV and summary only; RGBA frame payloads remain excluded.
- `analysis-rerun-audit/`: pre-rerun analyses and the field-level audit summary.

The final harness patch and research conclusions are included. The live-production directory preserves the later implementation verification separately from the frozen research harness.

## Exact prototype snapshot

Apply `prototype.patch` in an owner-managed worktree at baseline `fd7c8f9f9fccb2a55f79f8ee87c25e2d5afd8d40`.
It includes the three exact source files in `source-snapshot/`.
Install `harness-inputs/` under `/private/tmp/stars-investigation/`, then compile the ignored research tests with `cargo test --release -p harmonigraph-render --no-run`.
Use source shaders for the experiments; the prototypes are not production corpus entries.
`frozen-harness-manifest.json` identifies the executable and sources preserved before work on the live comparison build.
No executable is distributed here.

`REPORT.md` is the investigation handoff.
P2 and P3 were accepted in stills and motion; the live implementation is measured and its comparison controls are ready for plugin evaluation.
The native-complete screen has image results but no performance measurement.
The optional occlusion screen exhausted its wall-time budget and produced no valid opacity percentages.
