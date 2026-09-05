# #615 evidence archive

The [measurement report](../../tuning-probe-bitwig.md) is the maintained account of the result and its limits.
This directory is a small index into the frozen investigation evidence, not another implementation of the probe.

Download the full archive, `MANIFEST.json` and `SHA256SUMS` from the [#615 evidence release](https://github.com/yan-h/harmonigraph/releases/tag/evidence-615-2026-09-04).
The release is marked as a prerelease to distinguish the experimental apparatus from a production plugin release.
It contains the original traces, numerical WAV captures, Bitwig project snapshots, configurations, action notes, historical analysis outputs and scratch analysis scripts.
No captured audio was played during testing or archival verification.

## Provenance and verification

`case-matrix.json` preserves the final compact case summary.
`run-index.json` records each available configuration, project snapshot and trace header/footer.
A zero-loss footer verifies capture completeness, not the musical interpretation of a run;
the report identifies intentional deadline faults, activation gaps, setup-only records and unobserved cases.
The historical analyses are retained unchanged, including superseded partial summaries and their original absolute paths.
Use the maintained report and final case matrix for the final verdict.

`MANIFEST.json` records the relative name, byte count and SHA-256 of every archived evidence file, plus the bundled source and archive README.
It excludes itself from its own inventory;
`SHA256SUMS` verifies the complete compressed archive and manifest.
The source bundle is a self-contained Git bundle at `a4a28369e1c358c9f8d205b3fca39a11ceaaa454`, before archive packaging and review fixes.
Its history includes the measured `637d4448`, `b664e1e3` and `efc1db7c` builds, as well as the intermediate diagnostic revision.
Later code changes do not retroactively change the source or results of those measurements.

After downloading all three assets into one directory:

```sh
shasum -a 256 -c SHA256SUMS
tar -xzf harmonigraph-615-evidence-2026-09-04.tar.gz
git clone harmonigraph-615-evidence/source/harmonigraph-615-source.bundle probe-source
git -C probe-source checkout a4a28369e1c358c9f8d205b3fca39a11ceaaa454
```

The archive also contains the contemporaneous report, probe instructions and carried-patch notes under `source/`.
The bundle preserves this repository's source history;
Rust dependencies, the pinned toolchain, Bitwig and third-party instruments/effects still need to be installed separately.
Follow the probe instructions at the selected source revision to rebuild an earlier experiment.

## Reanalysis and reproduction

The original archive root was `/tmp/harmonigraph-615-evidence`.
Scratch analyzers retain this literal path and some cross-run references.
Reanalyze a separate extracted copy by changing their `ROOT` assignment and any remaining absolute capture paths to the new location;
the scripts write their JSON results into that copy.
Keep the downloaded archive and its original summaries unchanged.
Most trace analysis uses Python's standard library;
scripts that also analyze audio, including the run-10 analyzer, require NumPy.
The oldest traces contain duplicate `sequence` keys, so use the historical analyzer's first-key handling where the report requires it.

The Bitwig snapshots cover the recorded per-run configurations, including the pitch ramp, same-key retrigger attempt, held and pending lifecycle cases, and the isolated Vital boundary trial.
They contain plugin state but do not bundle plugin executables.
Restore the relevant per-run probe configuration before activation and retain the report's hosting, buffer and clock-offset constraints.
Keep physical monitoring disabled and cue/preview volume at minimum for silent reproduction.
Recorded WAVs can be analyzed numerically without playback.

A second copy of the release assets is kept locally outside temporary storage and the managed worktree, under `~/Documents/Harmonigraph Evidence/615-2026-09-04/`.
This is an additional local copy, not a claim that a separate backup service has been configured.
