# FFT backend evaluation (#743)

Evaluation date: 2026-09-09.
Production source: `0c32443a5f23f59c5a4ad7fee21ec49628f8e436` (includes #800).
This is an experiment and a recommendation, not backend adoption or DSP crate extraction.
No production source, dependency manifest, lockfile, saved state or golden image changes.

## Recommendation

The planned RealFFT candidate merits a separate implementation proposal: it reduces complete stereo analyzer work substantially, especially for the shipped longer windows and multiple tapers, while offering a path to deleting the maintained FFT/twiddle/untangle implementation.
Keep the production backend as it is in this evaluation PR.
The numerical replacement is not bitwise equivalent, and rendered-frame/gate acceptance remains a prerequisite for any later adoption.
Do not treat these results as permission to bless goldens or close #743 as an adopted backend.

The default saving is about 0.067 ms per stereo column, approximately 0.83% of one core at the shipped 125 columns/second.
At 16384 samples/five tapers it is about 0.759 ms, approximately 9.48% of one core.
These are analyzer-path estimates from the measured paired differences, not measurements of total plugin CPU or offline export speed.
They supersede neither the issue's historical 0.043 ms transform component nor its scope: that component was never a current whole-analyzer benchmark.

Upfront integration effort was not a reason to reject the candidate.
The ongoing trade is a small dependency graph, CPU-specific numerical behavior, more retained planning memory and slower builds, against less numerical implementation to maintain and consistent measured CPU savings.
Decide the minimum DSP crate boundary separately if a dependency is actually selected: `harmonigraph-core` remains dependency-free and MIT OR Apache-2.0.

## Candidate and exact scope

[RealFFT 3.5.0](https://docs.rs/realfft/3.5.0/realfft/) is MIT licensed and wraps an even real input as a half-size complex transform plus postprocessing.
Its [planner](https://docs.rs/realfft/3.5.0/realfft/struct.RealFftPlanner.html) returns reusable plans; `process_with_scratch` uses caller-owned scratch.
[RustFFT 6.4.1](https://docs.rs/rustfft/6.4.1/rustfft/) is MIT OR Apache-2.0 and documents automatic AArch64 Neon selection, unnormalized transforms and ascending frequency order.
Both exact versions and all transitive checksums are pinned in the scratch lockfile; default SIMD features are enabled.
Package license/version fields were also checked in the downloaded registry manifests.

The [candidate patch](evidence/fft-backend-evaluation/candidate.patch) changes only a scratch copy of `SpectrumAnalyzer`'s transform storage, configuration and transform-to-bin-power loop.
It uses one plan per channel, drops the planner after configuration and retains input/output/scratch vectors with the plan.
It does not share plans across channels or retain a global cache.
Changing taper count replans, just as the baseline rebuilds its twiddle table; sharing or avoiding that work was not credited as a measured saving.

Window coefficients, f32 window products, ring traversal, usable bins `2..=N/2-2`, accumulated taper power, normalization, magnitude reconstruction, MAX-bin resampling, warm-up readiness, exact-zero shortcut and channel mean power are unchanged.
RealFFT also computes boundary bins that the analyzer discards; its extra boundary work is included.
The copied original FFT helper functions remain for their existing unit tests and are dead code in the candidate's production-style path.
A legacy helper's bit-exact test passing therefore does not establish candidate equivalence.

## Method and limits

The [reproducible evidence](evidence/fft-backend-evaluation/README.md) generates two standalone Cargo workspaces under a caller-selected scratch directory, outside production dependency resolution.
The baseline spectrum module is obtained directly from the pinned Git object without editing its analyzer path.
The candidate applies the tracked patch to that same object.
Both compile actual public `ChannelBank::push_frames` followed by `power_sum`, including deinterleaving, ring writes, both channels, every taper, normalization and pitch buckets.
No copied private transform benchmark determines the recommendation.

All nine shipped combinations are covered: 4096/8192/16384 samples and 1/3/5 tapers, at 48 kHz with a 384-frame hop.
Timing input is deterministic stereo: distinct harmonic/inharmonic tones per channel, amplitude modulation and seeded broadband noise.
Each analyzer is prefilled through hop-sized chunks, warmed for 32 columns, then measured over 500 nonzero columns or 2000 silence columns.
The 64-hop input cycle is identical for both binaries; its wrap discontinuity is part of this synthetic workload.
Outputs and input slices pass through `black_box`.

Seven fresh-process pairs alternate baseline/candidate then candidate/baseline.
The table reports medians and the range of paired savings, not a confidence interval.
Saved % is the median of the seven paired percentage savings, so it need not equal the ratio of the two separately reported timing medians.
The case order inside each process is fixed, so order/cache/frequency effects are not wholly randomized.
Construction and taper reconfiguration are measured separately: 25 fresh configured banks and 25 outward/back taper changes (50 changes).
Construction includes the public constructor's default setup before applying the requested size and taper settings; it excludes first-ever shared bucket-table initialization and includes bank destruction in each repeated-bank iteration.
Reconfiguration alternates one and three tapers for the one-taper rows, or one and the requested count otherwise, and reports the mean per change in each run.

An untimed separate bank lifetime counts requested heap bytes, including plans and transient allocations.
Construction peak is captured before feeding audio; retained bytes include reusable deinterleave capacity after hop-sized prefill.
An allocation probe checks a warmed public push/analysis call, then drops the bank and verifies all counted allocations were released.
Counters are disabled throughout every timed region; the allocator wrapper still reads a disabled flag on allocations, but performs no counter updates.
Requested bytes exclude allocator overhead, stack/returned bucket arrays, shared static frequencies and executable code.
This is not RSS.

Machine: Apple M1 Pro, 16 GiB RAM, macOS Darwin 25.6.0, native `aarch64-apple-darwin`, Rust 1.92.0 / LLVM 21.1.3.
Release settings match the relevant workspace choices: optimization level 3, LTO off, debug information stripped, default codegen units, no `target-cpu=native` override.
The coordinator explicitly paused #647/#742 builds, heavy tests and probes for accepted timings; read-only reviews continued.
Bitwig, its audio/plugin hosts and WindowServer continued running; the user session was not altered.
Per-run `ps` snapshots record this background load.
This is a window without competing builds, not an idle-machine laboratory result.
The standalone compilation unit may inline differently from the plugin; no x86/SSE/AVX, other Apple CPU, alternate sample-rate or end-to-end plugin/offline performance claim is made.

## Whole-analyzer results

| N | Tapers | Baseline µs | RealFFT µs | Paired saved µs (median; min–max) | Saved % |
|---:|---:|---:|---:|---:|---:|
| 4096 | 1 | 95.00 | 65.73 | 29.49; 26.87–31.97 | 30.9 |
| 4096 | 3 | 195.66 | 104.49 | 92.15; 87.25–124.62 | 47.4 |
| 4096 | 5 | 296.89 | 142.74 | 152.32; 147.12–156.61 | 51.6 |
| 8192 | 1 | 151.65 | 86.03 | 66.75; 61.44–71.25 | 43.4 |
| 8192 | 3 | 368.05 | 178.59 | 189.49; 185.19–197.13 | 51.6 |
| 8192 | 5 | 583.69 | 278.44 | 305.34; 296.99–320.74 | 52.0 |
| 16384 | 1 | 278.46 | 126.67 | 149.99; 146.25–167.96 | 54.2 |
| 16384 | 3 | 748.75 | 290.70 | 457.51; 453.02–460.36 | 61.0 |
| 16384 | 5 | 1213.95 | 454.88 | 758.60; 753.90–790.15 | 62.6 |

Exact silence remained exact zero in both implementations and returned through the unchanged shortcut.
Silence timings are essentially unchanged; no FFT acceleration is claimed for it.
All 252 untimed allocation probes (18 cases × 14 processes) reported zero allocations after prefill.

## Configuration, memory and build cost

| N/tapers | Retained bytes B/C | Construction peak B/C | Init µs B/C | Taper reconfigure µs B/C | Silence µs B/C |
|---|---:|---:|---:|---:|---:|
| 4096/1 | 165808.00 / 215840.00 | 344496.00 / 468840.00 | 136.12 / 213.87 | 80.80 / 103.20 | 8.16 / 8.08 |
| 8192/1 | 329648.00 / 428480.00 | 328112.00 / 427328.00 | 90.16 / 137.25 | 157.44 / 208.09 | 11.14 / 11.16 |
| 16384/1 | 657328.00 / 854816.00 | 672176.00 / 1001496.00 | 269.84 / 410.62 | 318.50 / 405.88 | 17.25 / 17.18 |
| 8192/5 | 591792.00 / 690624.00 | 623024.00 / 837176.00 | 453.80 / 556.47 | 223.24 / 273.04 | 11.19 / 11.29 |
| 16384/5 | 1181616.00 / 1379104.00 | 1245616.00 / 1673416.00 | 1001.50 / 1230.11 | 449.50 / 538.76 | 17.51 / 17.13 |

At defaults, the candidate retains 98,832 additional requested heap bytes per stereo bank (about 96.5 KiB).
At 16384/five tapers the additional retained allocation is 197,488 bytes (about 192.9 KiB).
Baseline transform vectors hold `8N` bytes per channel (split complex scratch plus shared FFT/untangle twiddles).
Candidate input/output vectors alone hold `8N+8` bytes per channel, plus its library plan/twiddles and any library-requested scratch.
Those remaining allocations are included in the measured bank total; this probe does not separately itemize the library plan's internal buffers.
There is no retained planner cache whose growth must be bounded.

The three alternating clean-build pairs took baseline **0.504/0.525/0.568 s** and candidate **8.759/8.535/8.657 s**.
The medians are **0.525 s versus 8.657 s**, an 8.132 s increase for this standalone project.
Executable sizes were stable in all three runs: **552,496 versus 1,072,320 bytes**, a 519,824-byte increase (about 508 KiB).
The candidate resolves nine external packages including RealFFT/RustFFT and their transitive/build dependencies.
The reserved run began at **22:40:52 UTC** and ended at **22:41:50 UTC** on 2026-09-09.

These are isolated executable and dependency-graph costs, not predicted full-plugin deltas.
The binary includes the same input generator and numerical probe in both variants; the production plugin has a different link graph and dead-code reach.
Fresh target directories disable rustc/sccache reuse, while the downloaded registry sources and operating-system disk cache remain warm.
Do not label this a cold-machine or dependency-download measurement.

## Numerical and picture implications

Verification covers the full shipped matrix over mixed stereo, exact silence, 440 Hz tone, anti-phase stereo tone, a quiet tone, seeded noise, periodic impulses and the exact pinned offline golden-audio generator.
The golden fixture runs for 192 columns after warm-up, reaching all three staggered 24-partial stacks and their broadband bed.
There is no checked-in recorded audio fixture; these are deterministic synthetic signals, not a music-corpus study.
Other cases run 16 consecutive columns.
All measured buckets were finite and nonnegative.

The independent reference directly evaluates an f64 DFT at 11 selected bins per channel after the last column, on the same f32-windowed samples, sums taper power, and compares both transforms independently.
Selected bins include the low endpoint, representative interior bins, the interpolation crossover, N/4 and the high usable endpoint.
It is a sampled-bin check, not an exhaustive independent reference for every bin or every output column.
Maximum error divided by the channel's peak bin power was `4.385e-7` for the baseline and `3.028e-7` for the candidate.
Maximum normalization-scaled absolute power error was `1.105e-7` and `7.048e-8`, respectively.
This supports comparable transform accuracy on the measured signals; it does not prove one library universally more accurate.

Across 10,473,408 paired output buckets, 8,167,899 differed as f32 values.
The largest absolute power difference was `3.278e-7`; the largest dB difference with either value above -120 dB was `0.1744 dB`, on the single-tone case with three tapers at N=4096.
The comparator floors values at `1e-12` for dB comparison, reports per-case RMS and maximum relative error, and does not hide low-power differences behind a single global relative tolerance.
Those descriptive thresholds are not a visual acceptance criterion.

The actual pinned `spectrogram::quantize` changed 1,457 bytes across the full suite, always by one byte (0.5 dB).
On golden audio alone, 84 of 6,614,784 bytes changed, with maximum float-power error `5.215e-8` and maximum above-floor dB error `0.02338 dB`.
The global peak-bucket winner changed in six golden-fixture columns; no peak-winner changes occurred in the other cases.
Exact silence had no float, byte or smoothed difference.

The power recurrence uses the exact pinned UI `hop_alpha` helper with default 10 ms attack/150 ms release and the same per-bucket rise/fall choice.
The largest smoothed power difference was `1.788e-7` overall and `2.980e-8` on golden audio.
This models the scalar recurrence over the tested columns, not the whole live/offline UI scheduling path or indefinite histories.

No candidate pixels were rendered, no lattice gate decisions were replayed, and no golden images were blessed.
The spectrogram shader interpolates stored bytes and maps them through a LUT, so a one-byte change can affect pixels beyond that bucket.
The curve applies logarithmic display mapping after smoothing.
The lattice audio ring additionally estimates local noise, smooths/rounds intensities and applies a gate with hysteresis: near a boundary, small input differences can toggle a node and persist through fades.
The relevant production paths are `harmonigraph-ui/src/panes/spectral_fold.rs`, `harmonigraph-scene/src/spectral.rs` and `harmonigraph-render/src/shaders/spectrogram.wgsl`.
An adoption proposal therefore needs unblessed full offline/lattice frame comparisons and deliberate gate-boundary fixtures on supported CPU backends, with explicit acceptance of any picture changes.
These untested effects are the reason this evaluation is not a ready-to-ship backend replacement.

Both isolated source-test suites passed 30 tests with one existing ignored timing table.
The candidate public-path tests include readiness/reset behavior, silence/resume, full-scale calibration, channel power and taper behavior; the independent DFT probe supplements the original helper-only FFT tests.
Production docs/format validation and the draft PR's CI are the appropriate checks for this evidence-only change.
