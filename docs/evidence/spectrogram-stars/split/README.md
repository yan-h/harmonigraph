# Production Stars split measurements

Measured on Apple M1 Pro / Metal,
macOS 27.0 (26A428),
on 2026-09-26.
The reference is the shader at `5d05abfd`;
the candidate is the native 2+3 split with specialized final pipelines in PR #1202.

## Results

The selected path saves about 14–17% at the dense defaults from 1440p through 4K in these runs.
1080p and 1296p select the unsplit fallback and remain effectively tied with the reference.
Low density (0.5) and both size endpoints at their minimum (0.5) also test the 1440p cutoff.
These benchmark fixtures fill the Stars region across the pane.
The final pass scissors partial combined panes to their drawn coverage,
and their smaller Stars area controls the cutoff.
The cutoff is a simple area policy based on this GPU,
not a universal hardware model.

Positive savings mean less GPU time.
“Production” selects the split automatically at 2560×1440 drawn pixels in area;
“Forced native” draws all five layers directly using the candidate shader's override-false pipeline.
The reference pair uses byte-identical copies of the original shader.
A/A is baseline B relative to baseline A.

| Run | Original ms | Production saving | Forced native saving | A/A difference |
|---|---:|---:|---:|---:|
| recording-1080-113 | 13.789 | -0.1% | -0.5% | +0.1% |
| recording-1296-101 | 13.710 | +0.3% | -0.1% | -0.1% |
| recording-1440-107 | 15.332 | +13.9% | +0.2% | -0.0% |
| recording-1800-109 | 22.170 | +16.6% | +0.3% | +0.2% |
| recording-4k-127 | 36.064 | +16.5% | -1.5% | +0.8% |
| small-stars-1440-137 | 15.723 | +15.0% | +0.2% | +0.1% |
| sparse-1440-131 | 12.383 | +18.7% | +0.4% | +0.1% |
| synthetic-1440-103 | 15.232 | +14.8% | +0.1% | -0.1% |

## Method and limits

Each run interleaves four cases for 120 warmup rounds and 240 measured rounds.
Every block of 24 rounds contains all 24 execution orders once,
shuffled by the recorded seed.
The analysis checks this balance before reporting the mean GPU interval.
Resources and color history remain independent per case,
and each case asserts exactly one initial full history upload.
The default color pickup and release remain enabled intentionally:
these are the user's dense Stars defaults.
The recording-derived input scrolls at 48 slabs/second and loops its source;
it is not a complete saved-project/analyzer replay.

The primary CSV `gpu_ms` interval runs from BEGIN of the real `spectral_cloud_source` pass to END of the final composite.
The source feeds blur,
material,
star memory/atlas,
and the final draw through texture dependencies.
The harness asserts that the source's begin and end stamps precede the composite end.
No measured frame reversed this bracket.
`begin_gpu_ms` duplicates that interval for compatibility with the scratch CSV layout;
other printed timing columns are diagnostics.
These are synchronous offscreen measurements,
not end-to-end Bitwig frame times.
Absolute timings remain variable;
the A/A controls and paired ordering bound some of that uncertainty,
not all of it.

An earlier run subtracted the END of an independent opening 1×1 timestamp pass,
as the reusable probe currently does.
That pass can finish after the final composite on this tile-based GPU.
The [rejected run log](rejected-independent-stamp.log) records an assertion failure,
and no measurements from that run appear in the table.
[Issue #1203](https://github.com/yan-h/harmonigraph/issues/1203) tracks correcting the reusable probe.
The original split prototype's older `end/full` evidence should be read with this limitation;
these final measurements use actual render dependencies.

## Reproduction

[Raw samples](.) are compressed without timestamps;
[environment files](.) retain each invocation's controls,
and [input hashes](input-sha256.json) identify the shader and recording bytes.
The recording input remains a local artifact.
Only the three synthetic cases can be reproduced from committed artifacts alone;
the other five require the non-committed recording input identified by its hash.

Apply this [scratch harness](harness.patch) to the PR's production source,
compile its release test executable,
and pass the executable path Cargo prints to the runner.
The runner reconstructs the reference WGSL from Git;
its otherwise-unused override declaration only permits the common pipeline constructor.
Do not build or load a plugin with the scratch harness applied.

```sh
git apply docs/evidence/spectrogram-stars/split/harness.patch
cargo test --release -p harmonigraph-render --lib --no-run
python3 docs/evidence/spectrogram-stars/split/run.py --binary /path/to/test-executable --output /private/tmp/stars-split-replay
python3 docs/evidence/spectrogram-stars/split/analyze.py /private/tmp/stars-split-replay
git apply -R docs/evidence/spectrogram-stars/split/harness.patch
```

Add `--recording /path/to/take-levels.u8` to reproduce all eight runs.
Run `analyze.py` without a path to check the committed measurements.
