# RealFFT adoption (#743)

This adopts the backend evaluated in [PR #803's report](fft-backend-evaluation.md).
Baseline: freshly fetched `origin/main` at `d34d66cfd5e739fbe2528e084ddab001bb4e8b4b`.
Yan requested merge after the documented numerical, picture and gate differences and comparisons were presented.
That instruction records acceptance of this behavior.
The three reviewed spectral goldens now record that accepted output.
No shared DAW slot was changed.

## Boundary and behavior

`harmonigraph-analysis` contains `SpectrumAnalyzer`, `ChannelBank`, window/taper generation and pitch resampling.
It retains the moved code's `MIT OR Apache-2.0` license and depends on core and MIT-licensed RealFFT 3.5.0 (RustFFT 6.4.1 in the lockfile).
Core keeps the fixed pitch axis, Hz/MIDI conversions and spectrogram history, with no dependencies.
Only UI constructs analyzers in production, so only UI gains the new dependency;
scene/render and math-only consumers still need just core's axis.
There is no compatibility re-export of the moved analyzer API.

The handwritten transform, twiddle generation and real-input untangling are deleted, along with their superseded implementation-specific tests and the example that copied those algorithms.
A maintained test now drives the configured analyzer through ring wrap and checks its actual accumulated bins against an independent f64 DFT.
The full supported 4096/8192/16384 × 1/3/5-taper matrix reaches the low/high usable endpoints, interpolation crossover and quarter-window bin.
This is a sampled-bin reference, not an exhaustive DFT of every output bin.

Each channel owns one planned real-input FFT, its input/output and library-requested scratch.
RealFFT retains the half-size complex transform plus real-input postprocessing.
The planner is dropped after configuration;
there is no shared planner cache or backend framework.
Windowing and f32 multiplication order, usable bins `2..=N/2-2`, normalization, taper accumulation, reconstruction, MAX-bin resampling, readiness, exact-zero shortcut and channel mean power remain unchanged.
Live, whole-song and offline consumers still share the same analyzer.
No saved fields, parameter ranges, analysis-quality settings or scheduling behavior change.

Configuration invalidation remains keyed on window size and taper count.
Both affect the retained window/tapers/normalization, and changing either still clears readiness;
sample-rate changes still clear the window while the existing bin conversion reads the new rate.
The FFT plan itself needs only size, but preserving the existing configure lifetime also replans on taper changes, as the evaluated candidate did.
This adoption does not narrow that invalidation key or introduce a newly reachable carry-forward path.
The existing immutable bucket-frequency table remains keyed only by the fixed pitch axis.

## Numerical and scene results

Before backend replacement, the extraction produced byte-identical f32 powers and quantized bytes across 10,473,408 paired buckets.
The integrated production-crate candidate reproduces the evaluation's numerical counts exactly:
8,167,899 differing floats, maximum absolute power difference `3.278255463e-7`, and maximum above-−120 dB difference `0.1743986 dB`.
There are 1,457 quantized-byte changes, all one level.
Golden audio contributes 84 of those across 6,614,784 buckets, with maximum power difference `5.215406418e-8`, maximum above-floor difference `0.02338449 dB`, and six global peak-winner changes.
These are bucket/byte results, not pixel tolerances.
Exact silence remains exact zero.

The [gate probe](evidence/realfft-adoption/gates.rs) pins input gains from the original FFT at adjacent f32 values on either side of an actual idle A node's strict and held `RingGate` decisions.
It uses the scene's own octave layout and Spectrum/Fold reading, not just a global peak or one chosen bucket.
A full configured window alone is insufficient for a scheduled analyzer:
each stage pushes through the next 384-sample hop so analysis actually runs.
Every history retains one `PictureState`, increasing clocks, stereo anti-phase audio and MIDI-idle nodes through quiet, strict-low, strict-high, inside-band, held-high, held-low and four silence stages.
Analyzer/ring smoothing is explicitly zero to isolate rounding and gate behavior;
node Fade is one second and linear.
The regular full-frame audio fixture below retains default smoothing.

All 18 baseline configurations reach real node transitions:
82 visible instances of the same A pitch class are closed at strict-low, open at strict-high, continue rising inside the band and fall below the held threshold.
The candidate changes 5,084 of 184,500 node/time observations in 12 of 18 configurations.
These are repeated instances of one class, not 5,084 independent notes.
Maximum node opacity difference is 1.0 in the 16384/1 and 16384/5 Spectrum cases:
missing a strict opening can keep the node closed throughout a following held-band history while the other backend fades fully in.
Differences survive into silence: up to phase 8 at 8192 samples, and phase 7 at 16384 samples.
This is expected threshold sensitivity to finite precision, but it is a visible behavior change explicitly accepted by Yan, not an error hidden by a tolerance.

The maintained `analyzed_audio_crosses_the_gate_and_carries_its_hysteresis_fade` regression drives the real analyzer and scene for default/largest settings in both readings, using a 2% margin around those measured boundaries.
It asserts an idle actual node's strict opening, rising held-band fade, partial closing fade and eventual silence.
The adversarial one-ULP observations stay in evidence rather than turning one CPU's rounding into a permanent requirement.

## Complete frames

Four full offline side-by-side histories contain both the lattice and spectral pane, including the live curve.
Each renders 192 frames at 640×400/60 fps:
the existing two-second golden tone/noise generator is made stereo (right channel offset 97 samples and scaled by 0.8), followed by 1.2 seconds of silence.
Default and largest/five-taper configurations each run Fold and Spectrum with the ring enabled, gate 0.5 and hysteresis 0.1.
Every RGBA byte is compared, including intermediate frames and fading rings.
The synthetic waveform's right-channel wrap is part of this fixed fixture;
this is not a recorded-music corpus.

| History | Changed frames / 192 | Changed pixels across all frames | Maximum channel change | Mean absolute channel change, 0–255 |
|---|---:|---:|---:|---:|
| Default / Fold | 172 | 578 | 76 | 0.000004700 |
| Default / Spectrum | 172 | 607 | 76 | 0.000004873 |
| Largest / Fold | 98 | 478 | 57 | 0.000003571 |
| Largest / Spectrum | 96 | 291 | 57 | 0.000002396 |

The changes are sparse, but a one-level stored-byte difference does not imply one-level pixels:
interpolation, mapping and curve geometry can produce a larger local difference.
[Default Fold](evidence/realfft-adoption/images/default-fold.png) and [largest Fold](evidence/realfft-adoption/images/largest-fold.png) show baseline/candidate/absolute difference ×32 for the frame with the largest summed difference.
The full CSV and additional sheets cover all four histories.

Separate ten-stage lattice renders use the exact baseline-pinned boundary histories.
[8192 / three tapers / Fold](evidence/realfft-adoption/images/boundary-8192-3-fold.png) and [16384 / five tapers / Fold](evidence/realfft-adoption/images/boundary-16384-5-fold.png) visibly change rings over six frames each:
maximum per-channel changes 13 and 25, with mean absolute channel changes 0.006791 and 0.025470 across the histories.
These are deliberately selected boundary cases, not typical-audio frequency estimates.
The [16384 / one taper / Spectrum](evidence/realfft-adoption/images/boundary-16384-1-spectrum.png) case whose node opacity differs by 1.0 changes five rendered frames, 50,856 pixels across them, with maximum per-channel difference 59 and mean absolute channel difference 0.032813.
Its most affected frame shows rings present in one backend and absent in the other.

## Integrated costs and limits

Machine: Apple M1 Pro, 16 GiB, native `aarch64-apple-darwin`, Rust 1.92.0, macOS Darwin 25.6.0.
The coordinator reserved the measurement window against competing builds/probes;
user apps remained running, and process snapshots are retained.
Release uses the workspace defaults (opt level 3, LTO off, no target-cpu override).
Compiler caching was disabled for both clean builds because the local sccache server returned EPERM.

Both actual plugin/offline artifacts are measured, not inferred from the isolated evaluation executable.
The original release plugin is 19,499,472 bytes and offline executable 16,504,784 bytes;
the candidate is 19,997,904 and 17,006,656 bytes, increases of 498,432 and 501,872 bytes respectively.
The baseline empty-target build took 88.98 seconds.
The candidate empty-target build took 88.29 seconds and produced the same artifact sizes as the precommit runtime pair.
This pair does not demonstrate a wall-time build penalty or saving.
The command-reported maximum RSS was 805,978,112 bytes for baseline and 980,779,008 for candidate.
One sequential clean-build pair is descriptive, not a reliable estimate of average build-time overhead;
registry downloads are excluded and OS disk caches remain warm.

Seven alternating fresh-process pairs render the actual offline executable to a raw RGBA sink backed by `/dev/null`, retaining render/submit/readback work and excluding video encoding and output-disk throughput.
Each process renders the same 192-frame history above, including device/pipeline startup and silence.
The fixed case order is default then largest;
variant order alternates B/C then C/B.

| Case | Baseline median seconds | Candidate median seconds | Median paired saving, seconds | Paired saving range |
|---|---:|---:|---:|---:|
| Default / Fold | 0.9682 | 0.9324 | 0.0022 | −0.0344 to 0.1067 |
| Largest / Fold | 1.1424 | 0.9913 | 0.1776 | 0.0969 to 0.2994 |

Default savings are not resolved above this workload's noise.
The largest configuration is faster in every measured pair.
Differences of the two timing medians are not the median paired saving.
Process peak RSS medians are 54,624,256/54,362,112 bytes at defaults and 55,066,624/55,033,856 bytes at largest (baseline/candidate):
this does not resolve a retained-memory difference, and excludes GPU residency.
No DAW/plugin FPS, callback CPU, plugin-host RSS or encoded-video speed measurement is claimed.

The workspace-linked public analyzer probe repeats the evaluation workload and allocator method, avoiding copied transform timings.
Median paired savings are 65.914 µs (63.683–67.123) per default stereo column and 767.463 µs (752.157–776.527) at 16384/five tapers.
Retained requested heap bytes are 329,648/428,480 at defaults and 1,181,616/1,379,104 at largest:
RealFFT adds 98,832 and 197,488 bytes per stereo bank.
All 252 warmed allocation observations report zero allocations;
all counted allocations are released on bank destruction.
These are requested heap bytes, including the plan and scratch, not RSS.
Construction peaks, initialization and taper-reconfiguration timings remain in the raw rows.

Native AArch64 uses the enabled RustFFT Neon backend.
A separate diagnostic build disables RealFFT's default features to execute RustFFT's scalar path;
its 21 analyzer tests (including independent DFT) and maintained scene regression pass.
Its baseline-pinned gate histories change 6,888 node/time observations in 16 of 18 configurations, with maximum opacity difference 1.0.
That demonstrates CPU-path sensitivity rather than universal bitwise equality.
Scalar full frames were not captured;
x86 SSE/AVX, other CPUs and other sample rates were not executed here.
The shipped manifest retains default SIMD features.

## Validation and acceptance

The [reproduction instructions and raw results](evidence/realfft-adoption/README.md) identify the temporary probes and captured artifacts.
No temporary module include, extra example executable, scalar feature override or copied original FFT remains in production.
The core dependency guard remains intact.
Workspace all-target clippy with warnings denied, strict private rustdoc checks, Markdown local links, formatting, and the isolated plugin package check pass.
The initial canonical `cargo test --workspace` run detected three unblessed spectral golden differences:
`a_short_pane_zoomed_out_draws_the_frame_on_record`, `a_tall_pane_zoomed_out_draws_the_frame_on_record`, and `the_whole_song_layout_draws_the_frame_on_record`.
The short and tall frames each change one channel of one pixel, and whole-song changes one channel at each of four pixels, all by 1/255;
[short](evidence/realfft-adoption/images/golden-spectrogram-short-pane.png), [tall](evidence/realfft-adoption/images/golden-spectrogram-tall-pane.png) and [whole-song](evidence/realfft-adoption/images/golden-spectrogram-whole-song.png) contact sheets retain the exact comparisons.
The zoomed-in and mixed-shadow spectral goldens pass.
A separate `cargo test --workspace -- --skip golden` passes 1,537 tests (29 ignored), and `cargo test -p harmonigraph-render golden` passes all 14 lattice checks.
The baseline passed all five spectral and 14 lattice golden checks.
After Yan accepted those comparisons, the repository-wide blessing command updated only these three PNGs.
A direct decoded-pixel comparison verified the exact expected one/one/four channel changes, each by 1/255;
all other goldens stayed byte-identical.
The subsequent unblessed `cargo test --workspace golden` passes all 20 checks (2 ignored).
The full `cargo test --workspace` passes 1,557 tests (31 ignored), including all five spectral and 14 lattice golden checks.

A `--release --workspace` test attempt hit a pre-existing `harmonigraph-record` test compile failure:
its global allocator references `nice_assert_no_alloc::AllocDisabler`, which that dependency's default `disable_release` feature removes in release builds.
The recording tests, manifest and dependency version are unchanged.
Required workspace validation uses the repository's normal dev profile instead.

Yan accepted the sparse representative pixel differences and potentially persistent ring differences at exact numerical thresholds on the measured CPU paths.
The platform limits and original comparison evidence remain unchanged;
final CI and merge coordination are recorded on the adoption PR.
