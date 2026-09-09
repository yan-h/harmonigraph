# Reproducing the #743 evaluation

This directory is evaluation evidence, not a production backend or a workspace member.
The [report](../../fft-backend-evaluation.md) contains the recommendation, measured values and limits.
Source is pinned to `0c32443a5f23f59c5a4ad7fee21ec49628f8e436`; run setup from a repository containing that Git object.
Requirements: Rust 1.92.0, Python 3, Git and `patch`.
Registry access is needed once to fetch the exact lockfile dependencies.

```sh
TASK_REPO="$PWD"
TASK_EVIDENCE="$TASK_REPO/docs/evidence/fft-backend-evaluation"
TASK_SCRATCH="$(mktemp -d /tmp/harmonigraph-fft.XXXXXX)"
TASK_RESULTS="$TASK_SCRATCH/results"
python3 "$TASK_EVIDENCE/setup.py" "$TASK_REPO" "$TASK_SCRATCH"
(cd "$TASK_SCRATCH/baseline" && cargo build --release --locked)
(cd "$TASK_SCRATCH/candidate" && cargo build --release --locked)
python3 "$TASK_EVIDENCE/run.py" "$TASK_SCRATCH" "$TASK_RESULTS" verify

# Reserve a window with no other heavy compilation or benchmarks first.
# Preserve the user's apps; document remaining background load.
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$TASK_RESULTS/window-start.txt"
python3 "$TASK_EVIDENCE/run.py" "$TASK_SCRATCH" "$TASK_RESULTS" bench
python3 "$TASK_EVIDENCE/run.py" "$TASK_SCRATCH" "$TASK_RESULTS" build
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$TASK_RESULTS/window-end.txt"
python3 "$TASK_EVIDENCE/summarize.py" "$TASK_RESULTS"
```

`setup.py` copies the exact spectrum and spectrogram modules from Git, applies `candidate.patch` only to the candidate project's spectrum module, and appends `probe.rs` to both spectrum modules.
It also extracts the golden-audio generator and `hop_alpha` from that revision, adapting only the golden generator's return container from `Audio` to `Vec<f32>`.
Each generated manifest has its own empty `[workspace]` and its own lockfile.
No root workspace manifest/lockfile is read or written by Cargo in this process.
Run setup into a fresh scratch directory; it overwrites generated sources if reused.

`bench.rs` contains the shared public-analyzer driver and untimed allocator probe.
The timing matrix and public setup sequence are identical between binaries.
The fixed atomic flag in the allocation wrapper stays disabled in all timed regions.
No backend-selector feature or production abstraction is introduced.

`run.py bench` starts 14 fresh processes: B/C, C/B, B/C, C/B, B/C, C/B, B/C.
Each CSV row is one within-process batch average, not a per-column distribution.
`summarize.py` calculates medians of those averages and paired savings ranges.
Do not average transform-only copied helpers into this result.

`run.py build` uses six initially absent target directories in order B/C, C/B, B/C, `--locked --offline`, and an empty `RUSTC_WRAPPER` to disable compiler caches.
Repeated invocation with the same clean target paths deliberately fails; use a fresh scratch directory for another build series.
OS caches and downloaded dependencies stay warm.

`run.py verify` emits little-endian f32 bucket dumps, byte dumps from the actual `spectrogram::quantize`, f32 smoothed values and sampled direct-DFT error rows.
`compare.py` compares the entire dump in the fixed source-pinned matrix order; 3828 is the pinned `SPECTRUM_BINS` value.
It reports errors rather than silently accepting a tolerance or blessing images.
The scalar recurrence uses the actual coefficient helper and default settings but is not a UI render.
Both copied-module test suites are run too; their old FFT-private tests still test the old helper even in the candidate binary.
The separate reference probe checks the candidate's populated bin powers after its actual public path.

The committed `results/` contains all 14 raw analyzer summaries, all six clean-build logs and values, two source-test logs, two direct-DFT tables and the full numerical comparison table.
Large regenerable float/byte dumps are intentionally omitted.
`background-load.csv` retains the 12 highest `ps` CPU entries from each pre-run snapshot, sorted descending by the reported percentage; these are observational process estimates, not precise instantaneous core occupancy.
No daemon or user application was stopped for the measurements.
`SHA256SUMS` pins the evidence sources, locks, raw summaries and logs.
`results/generated-SHA256SUMS` records the exact generated sources and timed release binaries.
Regenerating the final evidence into a second scratch directory reproduced every measured source file byte for byte.
CSV line endings are normalized to LF and test-log terminal blank lines trimmed in the retained evidence; values and test results are unchanged.

Independent methodology review preceded the accepted timing window.
Its fixes separated allocation counting from timings and separated construction peak from first-feed retained memory.
The coordinator owns subsequent review and merge decisions; this artifact does not authorize a production FFT change.
