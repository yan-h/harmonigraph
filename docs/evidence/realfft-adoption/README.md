# RealFFT adoption evidence

The [report](../../realfft-adoption.md) separates the original extraction, numerical replacement, actual frame/gate behavior and integrated costs.
Baseline production source is `d34d66cfd5e739fbe2528e084ddab001bb4e8b4b`.
Production uses RealFFT 3.5.0 / RustFFT 6.4.1, native AArch64 Neon;
a temporary scalar build disabled RealFFT default features.
None of these probes is a workspace member or compiled by the final production tree.

## Reproduce

Use clean owner-managed baseline and adoption worktrees.
Do not instrument the main checkout or load a DAW slot.
The scripts intentionally use the fixed scratch directory `/tmp/realfft-adoption`;
archive an earlier run before reusing it.
Python's frame comparison needs NumPy and Pillow.
Rust uses the pinned workspace toolchain, existing registry downloads and the machine's GPU.

Set `ADOPTION_EVIDENCE` to this directory in the adoption checkout.
From each target checkout, run `python3 "$ADOPTION_EVIDENCE/setup.py"`.
It generates the evaluation's exact pinned audio/ballistics helpers, attaches two test-only modules, and creates a temporary UI example that links the actual public analyzer crate.
It reuses the previous evaluation's benchmark driver while replacing its copied spectrum module with that public import;
its obsolete DFT output is removed because the maintained analyzer test now checks actual private bins independently.
The original experiment's README describes signal, warm-up, allocation accounting and timing-loop details unchanged here.

Build baseline release plugin/offline artifacts before instrumentation, from an initially absent target with `RUSTC_WRAPPER='' cargo build --release -p harmonigraph-plugin -p harmonigraph-offline` under `/usr/bin/time -l`.
Save both artifacts under `baseline/` in scratch.
Build the temporary example with `RUSTC_WRAPPER='' cargo build --release -p harmonigraph-ui --example adoption_analyzer` and save it as `baseline/analyzer`.
Run the offline `adoption_fixtures` ignored test once to write the fixed take/WAV inputs.
Run the UI `adoption_gate_histories` ignored test with `ADOPTION_OUTPUT=/tmp/realfft-adoption/baseline ADOPTION_CALIBRATE=1`.
This must finish its actual-node threshold assertions before keeping `gains.txt`.
The baseline dump after the minimum crate extraction was also compared with the original output using `cmp`, before changing any transform operation.

Build and retain the candidate artifacts/example identically.
Run its gate histories with `ADOPTION_OUTPUT=/tmp/realfft-adoption/candidate` and **without** `ADOPTION_CALIBRATE`, so the input gains stay fixed.
Run the offline `adoption_boundary_frames` ignored test on both variants using their matching `ADOPTION_OUTPUT` values.
The first two selected histories are 8192/3/Fold and 16384/5/Fold;
the third is 16384/1/Spectrum, a case whose node fade can differ by 1.0.
The ten rendered steps retain the same `PictureState`, ring settings, fixed gains and increasing sample clocks as the numerical node probe.

Run `frames.py baseline` and `frames.py candidate` to render all 192 intermediate frames of each representative case through the actual offline CLI, then `compare_frames.py` to compare every RGBA byte and write contact sheets.
The sheet picks the frame with the largest **sum** of differences, which need not contain the largest individual pixel difference.
The third panel is absolute difference ×32;
it is not what the user normally sees.
No golden is replaced by these scripts.

Each retained analyzer binary accepts `dump PATH` for f32 powers plus `.db` and `.smooth` files.
Run the earlier evaluation's `compare.py` with `baseline.bin`/`candidate.bin` and their suffixes pointing to those outputs.
The numerical CSV matches the earlier evaluation exactly, including the six changed global winners.
`nodes.csv` observes actual scene nodes and `gates.csv` additionally logs a selected bucket and a custom single-octave diagnostic;
claims about actual-node transitions come from `nodes.csv`, not that additional diagnostic.

Reserve a window without competing builds first.
`runtime.py` runs seven alternating baseline/candidate pairs per default/largest Fold case through the actual offline binary, timing complete processes and recording `/usr/bin/time` maximum RSS.
The raw sink is a `.rgba` symlink to `/dev/null`:
GPU readback stays in the workload, while encoding and output-disk speed do not.
Run seven alternating pairs of the analyzer executables without arguments, retaining each CSV as `analyzer-ROUND-VARIANT.csv`.
`summarize.py` produces per-setting analyzer and actual-node summaries.
Keep the user's apps running and record process load; the retained snapshots are observations, not a machine-idle guarantee.

For the scalar diagnostic, temporarily change only the RealFFT dependency to `{ version = "=3.5.0", default-features = false }`, rerun analyzer tests and pinned gate histories, save the feature tree, then restore the shipped manifest.
No scalar image or x86 execution is claimed.

Remove the inserted test module/include lines and temporary example before committing.
The final tree retains two relevant maintained regressions: actual analyzer-versus-DFT and actual audio-through-scene gate/fade history.
Run normal workspace tests, formatting, doc-link and core dependency checks.
A full workspace `--release` test command is not a substitute for the normal profile because the unchanged recording test allocator is configured out in release mode.

## Retained material

`results/` contains raw analyzer rows, integrated runtime rows, numerical comparisons, actual-node summaries, selected ring diagnostics, baseline gains, small process snapshots, feature coverage and validation/build logs.
Large node dumps and RGBA streams are reproducible rather than committed;
`generated-SHA256SUMS` identifies those measured files and binaries.
`images/` preserves the original baseline/candidate/amplified contact sheets from before acceptance.
After reviewing that evidence, Yan accepted the documented picture and gate behavior and requested merge.
Only the three reviewed spectral goldens were then updated with the repository blessing command;
`accepted-golden-deltas.csv` records their exact one/one/four channel changes, all by 1/255.
Subsequent unblessed golden and full-workspace checks verify the accepted state.

`measured-artifacts.csv` records sizes and SHA-256 values for the binaries actually used in the timing runs.
The baseline application artifacts were built from clean `d34d66c`;
the candidate measurements preceded the final commit and must not be identified by its later overlay tag.
The analyzer-source hash identifies the measured transform implementation specifically, not a claim that its whole precommit worktree was snapshotted.
To reconstruct that exact source, copy the final `crates/harmonigraph-analysis/src/lib.rs` to a scratch `lib.rs` and apply `measured-analyzer.patch` with `patch -p0` there.
The separately measured clean candidate build carries a full local source-file hash inventory in `clean-build-source-SHA256SUMS`;
its artifacts and later final tagged handover are distinguished in the report.
Powers, `.db`, `.smooth` and complete-frame hashes are in `generated-SHA256SUMS`.
