# A4: numerical analyzer and retained power

Investigator: audio_tuning.
Audited source: `ad1c6e6b9474dd80098703ae15cac241bba0eb41`.
Source references below are relative to the repository root at that revision.
After the first full crate path, `analysis/`, `core/` and `ui/` abbreviate `crates/harmonigraph-analysis/`, `crates/harmonigraph-core/` and `crates/harmonigraph-ui/`; `plugin/` abbreviates `crates/harmonigraph-plugin/`.
This leaf used source, fixture and historical-evidence inspection and independent arithmetic;
it ran no builds, benchmarks or host sessions.
The coordinator owns execution and integration.

## Scope and disposition

Status: deep for calibration structure, exact-zero behavior, multichannel power pooling, pitch-resampling bounds, Fold floor semantics and history coarsening;
the coordinator subsequently reproduced nonfinite-input recovery failure with the bounded scratch test below.

The finite-input paths examined have coherent ownership and numerical semantics.
No additional FFT backend, planner abstraction, precision framework or floor redesign earns its maintenance cost from this investigation.
This is a scoped clean result, not a claim that every frequency, amplitude, sample rate or floating-point boundary was exhaustively verified.
One low-priority robustness lead remains: a nonfinite audio sample appears able to poison the smoothed display after the raw analyzer has recovered.
No realistic upstream occurrence was established.

The fresh UI window is **Balanced, 8192 samples**, not Fast.
Fast, 4096 samples, remains selectable and saved Fast configurations remain Fast.
`docs/intent/spectrum-analysis.md:7–8` records both that decision and acceptance of Fast's loss of some low audio-ring fundamentals.
Precise is 16384 samples.
The calculations below include Fast because its shorter window exposes boundaries that the default cannot exercise at the same sample rate.

## Calibration and backend evidence

`crates/harmonigraph-analysis/src/lib.rs:705–766` generates tapers and normalizes their accumulated powers.
For taper weights `w`, the positive-frequency bin response of a unit sine away from boundary interference is approximately `sum(w)/2`.
The normalization is therefore `4 / sum_k(sum_i(w[k,i])²)` across tapers.
The single Hann case preserves its established f32 operation order, `(2 / sum(w))²`.
For the periodic Hann, `sum(w) = N/2` mathematically, so a bin-centered unit sine gives power `N²/16`, multiplied by `16/N²`, yielding one.
This derivation explains the scale;
it does not promise exact unity after arbitrary off-bin resampling or near the excluded DC/Nyquist boundaries.
Even-order sine tapers have nearly zero weight sums individually, so separately normalizing each taper would be an incorrect simplification.

The existing `a_full_scale_sine_reads_unity_at_every_taper_count` test (`analysis/src/lib.rs:1411`) reaches all counts 1 through 8 at 8192 samples, 48 kHz and a 2 kHz unit sine.
Its `N + 1234` samples, delivered in 701-sample chunks, fill and wrap the window.
It permits 1.5 dB peak error.
It is useful absolute-scale coverage, not an all-pitch or all-rate calibration guarantee.
The supported UI counts 1, 3 and 5 are within that fixture.
Tests immediately below it separately check the direction of noise-variance reduction and the raised noise floor from multiple tapers.
The extra noise-floor level is an estimator tradeoff, not evidence that the power normalization should be replaced.

`configured_analyzer_matches_direct_dft_after_ring_wrap` (`analysis/src/lib.rs:1290`) is stronger than a helper-only FFT test.
It runs the real analyzer for 4096/8192/16384 samples crossed with 1/3/5 tapers.
Each case feeds `N + 997` samples and asserts the actual ring write position is 997.
The independent f64 DFT consumes the analyzer's f32-windowed samples and compares accumulated powers at bins 2, 3, 7, 37, 75, 113, 301, 554, 555, `N/4` and `N/2-2`.
It reaches the first and last usable bins, the interpolation crossover and the quarter-window bin.
Its error is divided by the spectrum's global peak and must be below `1e-6`;
that is not a per-quiet-bin relative-error guarantee.
It checks sampled transform bins, not an independent oracle for every pitch bucket.

History already paid for a backend replacement: #803 evaluated it and #804 (`af9e2fc3e`) adopted RealFFT.
`docs/realfft-adoption.md:39–72` records accepted numerical and threshold-sensitive picture changes, including one-ULP gate effects.
Those historical measurements are evidence about that comparison, not fresh timings or current-machine measurements from this audit.
The historical document's original invalidation description was subsequently superseded by #1407 / `90dbf18c`;
the A2 investigation covers current resource dependencies.
Verdict: retain the current normalization and single backend.
Reopening the accepted backend choice would add work without a demonstrated new benefit.

## Pitch resampling and sample-rate boundaries

`analysis/src/lib.rs:230–269` plans each bucket as Silent, Loudest or Between.
`usable_bins` (`:467`) returns `2..=N/2-2`.
Loudest takes the maximum power of contained usable bins;
it does not sum bins and therefore does not gain level simply because high-frequency pitch buckets span more bins.
Between reconstructs magnitude and then squares it.
Its monotone cubic limits overshoot and avoids squaring an invented negative interpolation lobe into a bright phantom.

The lower usable-bin frequency is `2 * sample_rate / N`.
Representative arithmetic, in Hz:

| Sample rate | Fast 4096 | Balanced 8192 |
| --- | ---: | ---: |
| 22050 | 10.7666015625 | 5.38330078125 |
| 44100 | 21.533203125 | 10.7666015625 |
| 48000 | 23.4375 | 11.71875 |
| 96000 | 46.875 | 23.4375 |
| 192000 | 93.75 | 46.875 |

These values describe usable transform bins, not guaranteed audible-ring cutoffs.
Bucket placement, leakage, taper bandwidth, floor subtraction and gate settings still affect a particular tone.
The last usable frequency is `(N/2-2) * sample_rate / N`;
for Fast at 22050 Hz it is 11014.2333984375 Hz.
The planner clamps a bucket's upper edge to the last usable bin before choosing Loudest.
This preserves a bucket that overlaps the usable high endpoint rather than incorrectly sending a wide bucket to Between.
The sample-rate-change fixture at `analysis/src/lib.rs:1133` uses both 96000 and 22050 Hz and compares with a fresh analyzer.
It reaches changed rate and low Nyquist configurations, but both sides use the same planner, so it is not an independent proof of the planner formula.

The magnitude-prefix bound has a configuration-independent explanation.
At 32 buckets per semitone there are 384 buckets per octave.
A bucket centered at bin coordinate `x` has width `2*x*sinh(ln(2)/(2*384))` bins.
Its width equals one at approximately `x = 553.99482049`.
Away from excluded endpoints, an interval at least one bin wide contains an integer bin and takes Loudest.
The Between branch therefore needs at most floor-center 553 and its neighbor lookahead through bin 555.
`INTERP_BIN_CEILING` (`analysis/src/lib.rs:63`) is `floor(384/ln(2)) + 8 = 561`.
The fill at `:425–442` includes sufficient lookahead and explicitly supplies the held low endpoint.
This is a bound argument for the present fixed pitch grid, not a reason to retain that constant if the grid definition changes.

Fixture reach matters here:

- `the_lowest_buckets_draw_at_every_window_length` (`analysis/src/lib.rs:895`) actually covers Fast at 48/96 kHz and Balanced at 96 kHz, using a tone at bin 2.5 and `N + 1234` samples through its helper.
  It checks nonzero nearby pitch buckets where the first interpolation pair is reachable.
  The preceding comment's quoted Hz values are twice `2*rate/N`; the fixture itself uses the correct formula.
- `the_reconstruction_has_no_seam_at_a_bin` (`:922`) checks interpolation knots with asymmetric data.
  `the_reconstruction_never_overshoots_the_bins_it_reads` (`:946`) includes the null-next-to-rise case that ordinary cubics mishandle and a sweep of local shapes.
  These are reconstruction helper tests, not whole-pipeline calibration tests.
- The DFT fixture above reaches high and low usable endpoints for every shipped window size at 48 kHz.
  The reconstruction-versus-windowed-DFT comparison at `:1002` provides additional approximation evidence;
  it does not turn interpolation into exact high-resolution frequency measurement.

Verdict: clean within the examined bounds.
Retain the monotone reconstruction and endpoint treatment.
The stale Hz comment is a small documentation correction when this area is next edited, not grounds for a numerical rework.

## Exact-zero windows and independent channels

`analysis/src/lib.rs:359–370` checks readiness before its exact-zero shortcut and examines the entire retained ring, not just the newest block.
It returns a real zero measurement, allowing history and display decay to progress.
Later nonzero transforms overwrite their input, accumulated powers, required magnitudes and every output bucket.
There is no finite-input stale-scratch dependency across the shortcut.

`silent_windows_keep_readiness_retained_audio_and_resume` (`analysis/src/lib.rs:825`) uses Fast's 4096 samples with both 1 and 8 tapers.
It checks signed-zero input one sample short of readiness, completion, a nonzero impulse at `N-2` rather than the erased Hann endpoint, 17 zero samples crossing the ring seam while the impulse remains, full replacement by zeros, and resumption with amplitude `1e-10` audio.
The fixture reaches the distinctions the optimization relies on.
No silence-threshold approximation is present, and no new denormal/silence mechanism is proposed.

`ChannelBank::power_sum` (`analysis/src/lib.rs:654–668`) analyzes channels independently and averages their powers.
For finite input, `[x,-x]` and `[x,x]` produce the same mean power;
`[x,0]` produces half the centered stereo power, or `10*log10(1/2) = -3.0102999566 dB`.
A mono mixdown would instead erase antiphase content.
The tests at `:1231` and `:1257` exercise antiphase preservation, mono/centered agreement and the single-sided case at the default 8192/48 kHz configuration.
Their fixture is not a rate/window matrix, but the independent-channel mean itself has no pitch-dependent branch.
Verdict: retain this ownership and pooling rule.

## Floor subtraction and real gate reach

`crates/harmonigraph-ui/src/panes/spectral_fold.rs:239–300` estimates a local median over approximately ±150 cents, sampled every fourth bucket, giving 25 samples.
It evaluates the floor every eighth bucket, interpolates that floor in power, subtracts it with a zero floor, and applies a Gaussian mean normalized by the weights actually used.
Normalization at the axis edges uses the available weights.
These operations explain why a broad low-frequency lobe can be treated as floor by Fast even when its raw analyzer buckets are nonzero.
The owner explicitly accepted documenting this limitation in `docs/intent/spectrum-analysis.md:8`.
No new pitch estimator, lower-frequency exception or adaptive floor is justified by reproducing that accepted behavior.

The maintained `analyzed_audio_crosses_the_gate_and_carries_its_hysteresis_fade` fixture (`spectral_fold.rs:1584`) is broader than the old adoption report's default/largest wording.
It now runs Fast/One, Balanced/One and Precise/Five, each in Fold and Spectrum modes.
Each phase feeds `ceil(N/384)*384` frames at 48 kHz, ensuring a scheduled hop sees a full configured window.
It uses actual 880 Hz antiphase stereo through `AudioSpectrum::push_samples`, a real MIDI-idle A node and retained scene state across strict opening, hysteresis, closing fade and silence.
The input gain brackets previously measured thresholds by 2%, and analyzer/ring smoothing is zero to isolate gate behavior.
Thus it reaches the current fresh default and Fast, rather than passing before their first measurement.
It is not evidence for low fundamentals, every sample rate, normal smoothing or one-ULP backend equivalence.
The historical one-ULP sensitivity remains explicitly accepted evidence, not a missing tolerance to tighten.

## History power and represented sample counts

`crates/harmonigraph-core/src/spectrogram.rs:71–77` merges columns by adding linear power sums and represented counts, with a count-weighted time.
It does not repeatedly quantize old data.
`crates/harmonigraph-ui/src/spectrogram.rs:643–655` adds those sums in f64 and divides by the total original count once before quantization.
This avoids giving one coarse mean the same weight as one fine measurement.
For example, equally represented powers 1 and `1e-6` average to 0.5000005, about -3.0102956 dB;
averaging their dB values would incorrectly give -30 dB.

The core conservation test (`core/src/spectrogram.rs:406`) feeds 8192 columns, explicitly asserts an oldest count of at least four, and checks conserved total sample count and power.
The full-refold test (`ui/src/spectrogram.rs:1716`) also feeds 8192 columns and requires both a coarse oldest column and a newest count of one.
It compares the resulting one-slab value against the total original power divided by 8192, with a quiet bucket checked separately.
These fixtures actually cross tier boundaries.

With 2048 fine columns, six 1024-column coarse tiers and an 8 ms hop, the full representation holds `2048 + 1024*(2+4+8+16+32+64) = 131072` original sample columns, or 1048.576 seconds.
That exceeds the UI's 610-second retention requirement (`ui/src/spectrum.rs:459`), so nominal representation capacity does not silently shorten the supported 600-second view.
This arithmetic is about retained history, not a newly measured memory or runtime cost.
At the largest tier, a stored column represents 64 original measurements.
Finite f32 sums still incur ordinary rounding;
the design does not claim exact real arithmetic.

`core/src/spectrogram.rs:26–27` uses a -120 dB floor and 0.5 dB byte step, giving at most 0.25 dB rounding error within the unsaturated interior and an upper representable level of +7.5 dB.
Coarsening moves timestamps to group centroids, so a later refold can differ at slab boundaries from an incremental raw-arrival fold.
That is the documented timing-resolution tradeoff (`core/src/spectrogram.rs:1–5`), not lost sample-count weighting.
#884 / `46e6a3b1` paid for the linear-power representation;
the present tests protect that behavior instead of its former mechanism.
Verdict: retain power sums and counts; reject repeated-dB averaging or a new exact-history layer.

## Pending robustness probe: nonfinite input and display recovery

Status: reproduced by the coordinator; see the execution outcome below.
This is outside the finite-input clean conclusion above.
There is no established occurrence in actual host audio or ordinary saved projects, so priority is low unless execution or real input evidence changes the assessment.

`analysis/src/lib.rs:318–324` stores samples without a finite-value gate.
The inspected live ingress and editor forwarding path likewise copies audio rather than sanitizing it (`plugin/src/audio_ingress.rs:81–106`; `plugin/src/editor/input.rs:81–103`).
NaN is not exact zero, so a poisoned retained window reaches the transform.
Some Loudest buckets can discard NaN through `f32::max`, while Between reconstruction can return nonfinite powers.
`ui/src/spectrum.rs:422` then updates retained display state with `shown += (new-shown)*alpha`.
Once `shown` is NaN, finite future `new` values cannot repair it through that equation, even when alpha is one.
The display accessor (`:455`) checks input recency, not finiteness.

Existing gates do not cover that retained state:
`hop_alpha` (`ui/src/spectrum.rs:184`) checks timing configuration;
`SpectrogramColumn::from_power` (`core/src/spectrogram.rs:61`) replaces nonfinite or negative history values with zero;
Fold's zero clamp can hide some bad values downstream without repairing the display buffer.
History can therefore look finite while the smoothed display remains poisoned.
No inspected existing fixture supplied a nonfinite audio sample and then checked recovery after clean windows.

The following bounded scratch test is proposed for the existing `crates/harmonigraph-ui/src/tests/spectrum.rs` module.
It uses the real public feed and display accessor, not private mutation of the display array.
It makes no permanent test addition recommendation until its result is read.

```rust
#[test]
fn audit_display_recovers_after_one_nonfinite_audio_sample() {
    let mut spectrum = AudioSpectrum::default();
    let config = SpectrumConfig {
        window: SpectrumWindow::Balanced,
        attack: 0.0,
        release: 0.0,
        ..SpectrumConfig::default()
    };
    let n = config.window.samples();
    let mut poisoned = vec![0.0; n + 384];
    poisoned[n / 2] = f32::NAN;
    spectrum.push_samples(&poisoned, 1, 48_000.0, 1.0, &config);
    let bad_before = spectrum.display(1.0).unwrap().iter().filter(|p| !p.is_finite()).count();
    let clean: Vec<f32> = (0..2 * n)
        .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
        .collect();
    spectrum.push_samples(&clean, 1, 48_000.0, 2.0, &config);
    let last = spectrum.history().back().expect("clean input emitted columns");
    assert!(last.power_sum.iter().all(|p| p.is_finite()));
    assert!(last.power_sum.iter().any(|p| *p > 0.01), "last FFT measures the clean tone");
    let shown = spectrum.display(2.0).expect("clean input is flowing");
    let bad_after = shown.iter().filter(|p| !p.is_finite()).count();
    eprintln!(
        "nonfinite display buckets: before={bad_before}, after two clean windows={bad_after}"
    );
    assert_eq!(bad_after, 0, "a past nonfinite input must not permanently poison the display");
}
```

The two clean windows are deliberately more than needed to replace the poisoned retained samples and reach a subsequent scheduled hop.
The tone-power assertion distinguishes actual recovered measurements from history merely sanitizing every bucket to zero.
Expected code-level outcome is a failed final assertion, but no measured count or confirmed reproduction is claimed here.
Next action: coordinator executes the isolated test and records the output, then restores scratch source.
If confirmed, investigate one shared finite-value boundary whose behavior can be stated simply;
defer a broader recovery protocol, health state, configuration option or exceptional-input scan.
Any repair must justify its per-sample/per-bucket cost and ongoing contract against actual reach.

## Engineering assessment

Current finite-input numerical responsibilities are separated usefully: the analyzer owns calibrated raw power, channels pool power, the UI owns temporal smoothing, Fold owns local excess-power reading, and history owns weighted retention.
Combining those states would erase real semantic differences rather than remove duplicated work.
The strongest useful results here are verified fixture reach and explicit limits on what each oracle proves.
The executed NaN experiment settles its stated state-retention claim without inventing runtime-cost evidence or reopening accepted Fast/RealFFT decisions.

## Coordinator execution outcome

The preserved Fast/48kHz scratch test reproduced the nonfinite recovery defect: 2564 display buckets remained nonfinite after two clean windows, while the newest history was finite and lit.
See logs/probe-nonfinite.log and DECISIONS.md for scope and the low-priority local-guard recommendation.
This does not establish real-project incidence.
