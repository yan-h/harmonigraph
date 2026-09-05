# #615 evidence archive

The [measurement report](../../tuning-probe-bitwig.md) is the maintained account of the result and its limits.
This directory is a small index into frozen evidence, not another implementation of the probe.

The filtered package contains the original numeric traces, synthetic test WAVs, probe configurations, action notes, historical analysis outputs and scratch analysis scripts.
Host logs and Bitwig project files, including their embedded plugin presets, are excluded from the public package.
The complete original archive is retained locally in `~/Documents/Harmonigraph Evidence/615-2026-09-04/`.
The filtered assets have a second local copy in that directory's `public/` subdirectory.
No public copy has been published.
These are additional local copies, not a claim that a separate backup service has been configured.

## Provenance and verification

`case-matrix.json` preserves the final compact case summary.
`run-index.json` records each available probe configuration and trace header/footer, plus counts of project snapshots held only in the private archive.
A zero-loss footer verifies capture completeness, not the musical interpretation of a run;
the report identifies intentional deadline faults, activation gaps, setup-only records and unobserved cases.
The historical analyses are retained unchanged, including superseded partial summaries and their original generic `/tmp/harmonigraph-615-evidence` paths.
Use the maintained report and final case matrix for the final verdict.

The public package preserves the selected original files byte for byte.
All 12,100,700 trace records were parsed for a string-value audit;
their string values consist of fixed diagnostic labels and the four recorded probe build tags, as listed in `trace-string-audit.json`.
The trace records retain numeric runtime process/thread identifiers and opaque host addresses used by the experiment.
The accompanying configurations, summaries, notes and scripts were checked for personal home-directory paths and credential/email patterns.
The WAVs are recorded synthetic instrument output from the test fixture;
no captured audio was played during testing or archival verification.
The source bundle contains the repository's already-public Git history.

`MANIFEST.json` records the relative name, byte count and SHA-256 of every public package member other than itself.
It also records the categories and counts omitted from the private archive.
`SHA256SUMS` verifies the complete compressed archive and manifest.
The self-contained source bundle is at `a4a28369e1c358c9f8d205b3fca39a11ceaaa454`, before archive packaging and review fixes.
Its history includes the measured `637d4448`, `b664e1e3` and `efc1db7c` builds, as well as the intermediate diagnostic revision.
Later code changes do not retroactively change the source or results of those measurements.
`source/LICENSE.nice-plug` preserves the upstream ISC notice alongside the historical bundle, whose vendored dependency snapshot predates the notice being added to the repository.

With the archive, `MANIFEST.json` and `SHA256SUMS` in one directory:

```sh
shasum -a 256 -c SHA256SUMS
tar -xzf harmonigraph-615-public-evidence-2026-09-04.tar.gz
git clone harmonigraph-615-public-evidence/source/harmonigraph-615-source.bundle probe-source
git -C probe-source checkout a4a28369e1c358c9f8d205b3fca39a11ceaaa454
```

The archive also contains the contemporaneous report, probe instructions and carried-patch notes under `source/`.
Those historical documents predate this public/private archive split;
their references to project snapshots and host logs refer to the complete private archive.
The source bundle preserves this repository's history;
Rust dependencies, the pinned toolchain, Bitwig and third-party instruments/effects still need to be installed separately.

## Reanalysis and reproduction

Scratch analyzers retain the original literal `/tmp/harmonigraph-615-evidence` root and some cross-run references.
Reanalyze a separate extracted copy by changing their `ROOT` assignment and any remaining absolute capture paths to the new location;
the scripts write their JSON results into that copy.
Keep the archive and its original summaries unchanged.
Most trace analysis uses Python's standard library;
scripts that also analyze audio, including the run-10 analyzer, require NumPy.
The oldest traces contain duplicate `sequence` keys, so use the historical analyzer's first-key handling where the report requires it.

The public source includes the MIDI fixture generator and probe configuration instructions.
The per-run configurations and report describe the pitch ramp, same-key retrigger attempt, held and pending lifecycle cases, and the isolated Vital boundary trial.
Exact saved DAW project restoration requires the private project snapshots;
the public package supports reanalysis and rebuilding the documented fixture without publishing those plugin states.
Restore the relevant probe configuration before activation and retain the report's hosting, buffer and clock-offset constraints.
Keep physical monitoring disabled and cue/preview volume at minimum for silent reproduction.
Recorded WAVs can be analyzed numerically without playback.
