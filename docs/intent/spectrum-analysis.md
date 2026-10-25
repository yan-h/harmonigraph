# Intent: Spectrum analyzer and audio analysis

Yan’s decisions recorded for future drift audits.

## 2026-10-02 (issue sweep)

- Balanced (8192 samples) is the fresh Frequency resolution again. Saved Fast selections remain Fast. #1314 Analysis Q1 (replaced 2026-10-07, below)
- Document Fast’s loss of low audio-ring fundamentals rather than redesigning the noise floor. The cutoff depends on sample rate and material; harmonics may still light the pitch class. #1314 Analysis Q2

## 2026-10-07 (DAW look capture, #1475)

- Fast (4096 samples) is the fresh Frequency resolution, captured with the rest of the DAW look; this replaces #1314 Analysis Q1.

## 2026-10-22 (look prototype, #1560)

- Sweep is a fourth Frequency resolution, beside Fast, Balanced and Precise rather than replacing them; Fast stays the fresh setting.
- Its listen glides log-linearly across the whole audible range, 20 Hz to 20 kHz, with no band edges: piecewise multi-resolution bands were picked on the sheets and then rejected for their seams.
- The live edge waits for the bass, so a column stays aligned across the whole axis; its latency is Precise's.
- No spectral reassignment, alone or on top of a sweep.
- Two sliders, bass and treble listen, start exposed and are to be pruned after use.

## 2026-10-23 (PR #1572)

- A video export draws the spectrum in step with its soundtrack: it reads half a window past each frame, so the newest column sits at the now-line rather than half a window behind it, at every resolution. The live view keeps its wait, since it cannot read ahead.

## 2026-10-23 (after #1572)

- No multitaper averaging: the analyzer reads one Hann window (or Sweep's kernels over it). Averaging steadied noise-like sound but widened every partial, raised the floor toward the lines, cost two to four times per column, and did almost nothing under Sweep's short treble listens.

## 2026-10-23 (analysis truth audit, #1575)

- Every frequency resolution reads each pitch bucket at its own frequency, so a partial is drawn at its pitch rather than on the nearest FFT bin. Fast, Balanced and Precise read a frame half as long again as their old Hann windows (6144, 12,288 and 24,576 samples) through one Gaussian listen near its newest end, matched to the old window's sharpness and lag, on a transform zero-padded to twice the frame. Chosen over doubling each frame (the same look at twice the live lag), a centred listen as long as the frame allows (the window's sidelobes drawn as false peaks around every loud partial), a shorter centred listen (clean, but every peak 28% wider) and placing peaks on the old bins (which assumes one partial per lobe). Columns are stamped where their window's weight sits, so the live lag and the export's read-ahead follow the listen rather than half the frame.
- Each pitch keeping its own lag under Sweep (treble drawn sooner than bass) was reconsidered and left as decided on 2026-10-22: the live edge still waits for the bass.

## 2026-10-24 (#1600)

- Sweep is removed; Fast, Balanced and Precise are the Frequency resolutions. At the lag Precise has, no window a Sweep could glide through drew a bass as sharp as Precise's without false peaks: Hann-tapered Gaussians draw lobes in bands at in-between lengths, and gamma windows stay clean but wider in the skirt. What remained was a faster treble, which did not earn a fourth mode. A saved project set to Sweep opens at defaults.
- No cheap lower lag for the fixed resolutions: moving the listen nearer the newest end saves at most 14% (Precise 172 → 148 ms) at skirts up to 1.6× as wide, the same trade a shorter listen makes.
- Frequency resolutions are lengths of time, not sample counts: the frame doubles at 88.2 and 96 kHz, so a setting looks and lags the same at every rate. Counted in samples, 96 kHz read every setting twice as blurry at half the lag, and Fast lost 20 to 44 Hz.
