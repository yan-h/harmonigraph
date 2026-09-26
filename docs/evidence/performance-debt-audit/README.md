# Performance and technical-debt audit evidence

These are one-off witnesses for the [2026-09-26 audit](../../performance-debt-audit-2026-09-26.md),
not production changes or an additional maintained test suite.
Audited source: `b26fa162`;
macOS arm64,
Rust 1.92.0.

From a checkout of the audited revision,
run one witness with an empty scratch directory outside the repository:

```sh
python3 docs/evidence/performance-debt-audit/run.py confirmed "$PWD"
python3 docs/evidence/performance-debt-audit/run.py motion "$PWD"
python3 docs/evidence/performance-debt-audit/run.py fixtures "$PWD"
python3 docs/evidence/performance-debt-audit/run.py png "$PWD"
```

The driver creates a temporary Cargo project,
seeds its dependency resolution from the checkout's lockfile,
and builds offline with the compiler wrapper disabled.
It writes no repository source.
Dependencies must already be available locally.
An optional third argument chooses an empty scratch directory.
Timing modes require macOS because their process CPU clock assumes Darwin's microsecond ticks.

| Witness | Source and output | What it establishes |
| --- | --- | --- |
| Confirmation | [confirmed.rs](confirmed.rs), [confirmed.txt](confirmed.txt), [turnover.txt](turnover.txt) | Imports actual State/Event modules and core, stubbing only the external input enum; repeats the Hub confirmation call and its begin/flush cadence. Demonstrates incomplete-state erasure, validates the source-turnover constraint, and prices repeated replacement. Does not run a host callback. |
| Motion | [motion.rs](motion.rs), [motion-first.txt](motion-first.txt), [motion.txt](motion.txt) | Calls real scene/motion code over windows derived by the production camera function. Times motion only, with ten warmups and thirty measured frames. The short-note bursts are synthetic stress. |
| Profile fixtures | [fixtures.rs](fixtures.rs), [fixtures.txt](fixtures.txt) | Shows immediate release on delivery of a future-dated Off, and equal drawn windows under the profiler's two named larger cases. The chosen aspect is illustrative, not an instrumented dock measurement. |
| PNG ownership | [png.rs](png.rs), [png.txt](png.txt) | Calls the real sink for three frames and then one frame to the same scratch path, exposing stale trailing files. No GPU or ffmpeg. |

[ui-profile.txt](ui-profile.txt) records the existing headless release profiles:

```sh
RUSTC_WRAPPER='' cargo test --release -p harmonigraph-ui -- \
  --ignored --nocapture --test-threads=1 \
  profile_frame profile_allocations profile_visual_runtime
```

All three passed.
Their busy-note and larger-lattice fixture limitations are findings in the report;
do not use those labels to claim they measure held-note workloads or 3075 drawn nodes.
The held-chord `profile_visual_runtime` scenario is separate and usable within its headless scope.

The CPU probes deliberately separate process CPU time from elapsed time because concurrent local work produced large scheduling delays.
They exclude GPU execution and DAW scheduling.
Process CPU time excludes descheduling but still varies with CPU frequency and cache contention.
Absolute numbers vary across runs;
the evidence supports the identified work and scaling,
not a fixed latency guarantee.
No proposed product optimization was applied,
so these are baseline measurements rather than demonstrated speedups.
