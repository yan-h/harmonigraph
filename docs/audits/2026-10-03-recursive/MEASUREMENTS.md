# Measurement provenance and limits

All source and executable claims refer to ad1c6e6b9474dd80098703ae15cac241bba0eb41 unless explicitly stated otherwise.
Scratch tests add assertions only; no production optimization was implemented.
The later piano-roll merge #1431 means roll-inclusive results must be rerun before attributing them to current main.

Machine: Apple M1 Pro, 8 CPU cores (6 performance, 2 efficiency), 14 GPU cores, 16 GiB RAM, macOS 27.0.1 (26A434).
Rust 1.92 release settings come from Cargo.toml; normal offline export has its package feature set, while workspace tests unify development features.
Debug-only allocation hooks are not established by the passing release suite.
The installed Bitwig and game remained running and untouched.

## Unmodified baseline validation

The complete workspace release run passed 1910 tests with 0 failures and 27 ignored tests.
Metal and FFmpeg were required; golden baselines were not changed.
The independent JavaScript adaptive reference model passed 13 tests.
The build, full per-test output and machine-readable count summary are preserved in logs/.
Ignored tests are not counted as passes or treated as universal coverage.
This run does not establish native Bitwig behavior or allocation freedom from debug-only guards.

## Completed encoder-inclusive setup samples

These are two successful single samples before the corrected three-round series, not statistically stable medians.
Both render the same deterministic eight-second six-voice progression with current default appearance and real x264 medium encoding, audio muxing and final completion.

| Workload | Process wall seconds | Renderer-reported total seconds | UI+tess ms/frame | Submit ms/frame | Readback ms/frame | Emit ms/frame | Process max RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 480 frames, 1280×720, 60 fps | 5.343 | 4.8 | 0.77 | 1.93 | 6.40 | 0.15 | 333.05 |
| 480 frames, 1920×1080, 60 fps | 8.495 | 8.4 | 1.03 | 2.42 | 9.30 | 2.55 | 717.81 |

Process RSS is /usr/bin/time's renderer process maximum, not the sum of renderer and FFmpeg or a measured GPU working set.
The readback stage includes waiting for submitted GPU work and unpadding; it does not isolate transfer cost or prove that pipelining can remove it.
The emit stage includes backpressure from the real sink, and total time includes final encoder completion.
No stage speedup is inferred from these baseline numbers.

Full output was inspected: FFmpeg reported guessed stereo channel layout and omitted edit lists; no appearance fallback or silently substituted setting was used.
The initial historical attempt refused a later repeated version-5 header; every scratch header was then explicitly converted while retaining its own audio start/sample rate.
That failed attempt is setup evidence, not a renderer bug or a performance sample.
The original take and WAV were never rewritten.

## Interrupted repeat

An identical 720p repeat was terminated after 138.128 seconds of wrapper wall time, with the renderer log reaching 444/480 frames.
The completed baseline was 5.343 seconds; the interrupted result has no final encoder completion and is censored, not a valid 138-second export result.
Process snapshots show the active game and desktop/DAW load; no competing audit compiler was running during that export.
The precise cause of the slowdown is inferred to be shared resource contention, not measured GPU attribution.
Only this audit's renderer and encoder were stopped, then their process tree was verified gone.

This variability prevents credible current GPU speedup claims or a useful three-sample spread under these conditions.
The common Otonal/Harmonigraph lock prevents our own expensive commands from overlapping; it cannot control unrelated applications.
Subsequent samples, if possible, are labeled separately and must not be pooled as one stationary distribution.

## Remaining narrow results

Hub, Namer, nonfinite display recovery and halo-only allocation assertions are pending the shared-lock probe build at this checkpoint.
CPU UI profiling and bounded follow-up exports may proceed only after audit-owned compilation ends.
See REPRODUCE.md for the exact protocol and HOST-VALIDATION.md for conditional native follow-ups.
