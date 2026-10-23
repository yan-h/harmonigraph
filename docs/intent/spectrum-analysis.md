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
