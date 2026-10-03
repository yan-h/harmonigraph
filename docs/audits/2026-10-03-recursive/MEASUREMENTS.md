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

## Completed repeats and workload inspection

External load returned before the queued retry, whose `settled` tag is only a run label.
The completed first repeats took 108.156 seconds at 720p and 97.577 seconds at 1080p; later runs in the same batch returned to roughly 5–7 seconds.
CPU preparation, submission, readback and encoder backpressure all varied, so the slowdown is not attributed solely to GPU transfer or rendering.
A final 1080p repeat was interrupted after 0.412 seconds while stopping redundant work; it is excluded from completed-sample statistics.
No unrelated process was stopped.

The following descriptive statistics include every completed sample, including the slow runs and the initial setup samples.
They are not a stationary performance distribution or a before/after comparison; the range is the key limitation, not an outlier to delete.

| Workload | Completed n | Wall median seconds | Wall min–max seconds | All wall samples seconds |
| --- | ---: | ---: | ---: | --- |
| chord-1280x720 | 4 | 5.542 | 5.328–108.156 | 5.343, 108.156, 5.328, 5.740 |
| chord-1920x1080 | 3 | 8.495 | 7.189–97.577 | 8.495, 97.577, 7.189 |
| historical-1920x1080 | 2 | 4.557 | 4.417–4.697 | 4.417, 4.697 |

The corrected historical workload completed twice without a schema/look fallback.
ffprobe verified the selected synthetic output as 480 H.264 frames/8.0 seconds and the historical output as 240 frames/4.0 seconds, both 1920×1080 with 48-kHz AAC audio.
Two local frames were inspected: lattice, roll, spectrum and nonempty spectrogram content render; this is a sanity check, not golden parity or proof of every recorded event.
The media and extracted frames remain local ignored artifacts; no original recording/media is committed.
Full stream evidence is in logs/media-inspection.json.

## UI CPU workload, three independent runs

The existing profile forces 600 measured frames after 60 warmups for each scenario, at 1600×1000 points and 2× pixel density.
Active cases hold six voices and feed 800 mono samples per frame at 48 kHz.
The timed region includes analyzer feed, UI construction and tessellation, but excludes sample generation and GPU callback preparation.
The preview case asserts that both lattice surfaces actually drew.
The test binary includes the scratch assertions, which are not executed by this exact profile filter; production code is unchanged.

| Scenario | Median of run p50, ms | Run p50 range, ms | Run p95 range, ms | Largest observed frame, ms | Median allocations/frame |
| --- | ---: | ---: | ---: | ---: | ---: |
| quiet dock | 0.155 | 0.154–0.155 | 0.219–0.249 | 0.590 | 721 |
| active dock | 0.675 | 0.654–0.688 | 0.889–1.026 | 8.962 | 985 |
| active dock + preview | 0.906 | 0.905–1.022 | 1.164–1.689 | 5.454 | 1400 |
| active no drawing | 0.064 | 0.064–0.065 | 0.098–0.100 | 0.225 | 2 |
| quiet no drawing | 0.000 | 0.000–0.000 | 0.000–0.000 | 0.000 | 0 |

Values displayed as 0.000 are rounded below the probe's printed precision, not proof of zero execution time.
The 8.962-ms active-dock maximum is preserved; this run does not attribute that isolated tail to code versus scheduling/background load.
Allocation counters are this UI harness's global counters, not the debug-only callback allocation guards.
“No drawing” calls shared begin_frame and is not a native closed-editor worker measurement; “quiet dock” forces frames and is not idle CPU percentage.
No native frame deadline or optimizer benefit is inferred.
These small median CPU costs do not justify another scene cache or snapshot owner on their own.

## Narrow probe results

- Native status query: ProcessStatus is 24 bytes/alignment 8, AtomicCell lock_free=false; u32 is 4 bytes/alignment 4 and lock_free=true. This queries the real cached dependencies; no callback timing claim.
- Hub: three full 1024-entry rows give 2048 popped/retained when matching, versus 3072 popped/0 retained when mismatched. This proves the accepted-count distinction, not an actual host stall or a tested candidate fix.
- Naming: actual MIDI 63.863136 spells Just E-, then equal-tempered fallback E after a failed current-node lookup. Full host held-note gestures remain unverified.
- Nonfinite input: one NaN leaves 2564 display buckets nonfinite after two clean 4096-sample windows, while newest history is finite and lit. The scratch recovery assertion intentionally fails with exit 101; the unmodified baseline suite remains green.
- Halo-only replacement: 30 alternating stable/change pairs at 1080p Uniform prove unchanged other allocation shapes and source-view replacement. CPU prepare median is 119.354 versus 372.292 microseconds, p95 153.166 versus 462.458; the 252.938-microsecond delta includes necessary halo allocation. Submit/poll are outside the timer. No complete-frame saving is demonstrated, so resource splitting is deferred.

Exact commands, exits, contexts and raw arrays are preserved in logs/probe-results.json, logs/probe-context.json and the individual probe logs.
All four temporary source additions were restored byte-for-byte before handoff.
The planned broad lattice-effect timing sweep was not run: unstable external load and the lack of a supported redesign candidate gave it low expected value after the native suite, complete exports and narrow allocation experiment.
This limitation is retained in the ledger rather than turned into a claim of comprehensive GPU performance coverage.
