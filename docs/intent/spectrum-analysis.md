# Intent: Spectrum analyzer and audio analysis

Yan’s decisions recorded for future drift audits.

## 2026-10-02 (issue sweep)

- Balanced (8192 samples) is the fresh Frequency resolution again. Saved Fast selections remain Fast. #1314 Analysis Q1 (replaced 2026-10-07, below)
- Document Fast’s loss of low audio-ring fundamentals rather than redesigning the noise floor. The cutoff depends on sample rate and material; harmonics may still light the pitch class. #1314 Analysis Q2 (Since #1604 frames are sized in time, so the cutoff no longer depends on sample rate, to within 9%; it still depends on the material. #1739)

## 2026-10-07 (DAW look capture, #1475)

- Fast (4096 samples) is the fresh Frequency resolution, captured with the rest of the DAW look; this replaces #1314 Analysis Q1.

## 2026-10-22 (look prototype, #1560)

- Sweep is a fourth Frequency resolution, beside Fast, Balanced and Precise rather than replacing them; Fast stays the fresh setting. (Removed 2026-10-24, below, with this entry's decisions on its listen, latency and sliders.)
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
- Each pitch keeping its own lag under Sweep (treble drawn sooner than bass) was reconsidered and left as decided on 2026-10-22: the live edge still waits for the bass. (Moot since Sweep's removal, 2026-10-24, below.)

## 2026-10-24 (#1600)

- Sweep is removed; Fast, Balanced and Precise are the Frequency resolutions. At the lag Precise has, no window a Sweep could glide through drew a bass as sharp as Precise's without false peaks: Hann-tapered Gaussians draw lobes in bands at in-between lengths, and gamma windows stay clean but wider in the skirt. What remained was a faster treble, which did not earn a fourth mode. A saved project set to Sweep opens at defaults.
- No cheap lower lag for the fixed resolutions: moving the listen nearer the newest end saves at most 14% (Precise 172 → 148 ms) at skirts up to 1.6× as wide, the same trade a shorter listen makes.
- Frequency resolutions are lengths of time, not sample counts: the frame doubles at 88.2 and 96 kHz, so a setting looks and lags the same at the studio rates, to within 9%. Counted in samples, 96 kHz read every setting twice as blurry at half the lag, and Fast lost 20 to 44 Hz.

## 2026-11-02 (drift audit, #1635)

- The spectrum/history now-line stays at the 2 pt line width, deliberately heavier than the 1 pt spectrum outline (#1585): the two share a colour, not a weight. #1635 Spectrum Q1, #1631
- Replace the defensive rule that drops a source-dated column not newer than the history's tail with a debug assertion: the clock only moves forward, so a break in ordering should fail tests loudly rather than be worked around in release builds. #1635 Spectrum Q2, #1631 (Superseded 2026-11-14 by the guard below, #1758; restored 2026-11-14 by S2, #1784: the assertion alone, with no release guard.)

## 2026-11-14 (premises audit, #1739)

Calls marked "delegated" were made by the audit session after Yan asked it to settle the decisions with an obvious answer; the reason is part of the line.

- Tape keeps its own note recorder joined by presentation time until Tape has been used in Bitwig; feeding it from the take lane is F3′'s Tape half, deferred while the take half goes to `propose-rework` (see `video-export.md`). #1739
- The analyzer reads a frame on request; drawing at once after a longer-frame switch holds its first column until it dates after the newest (or relies on the guard above). Part of F2′. #1739 (Not done: #1770 found the guard never drops or resets a column on a resolution switch, so this would remove nothing. #1758)

## 2026-11-14 (F2′ rework, #1758 PR A)

- Each feeder owns its analysis and dates its columns from a frame zero it states: the plugin from its clock mapping, the offline renderer from the first soundtrack frame it reads, the standalone synth from the frames it renders. Nothing dates columns by when they arrive, so the smoothed arrival anchor is gone, and the standalone synth's stall over 200 ms draws as a gap rather than sliding its columns back into line. After the plugin's input worker recovers from a crash, the newest edge keeps the lag of the resolution that drew it. #1758, #1761 (The lag half superseded 2026-11-14 by #1784 S3, below: the edge reads the lag from the settings.)

## 2026-11-14 (F2′ rework, #1758 PR B)

- The heatmap's history is one slab pyramid, a level per rung of the slab ladder, written once as each column arrives and never redrawn from stored columns. A rung change, a late open and the Video preview read what was folded on arrival. Answered yes to #1758 Q B; Yan, 2026-11-14.
- Under the slab pyramid, release builds drop a column not dated after the newest folded one, keeping the debug assertion and counting drops on the overlay (#1739 D10). A column more than a second behind the newest, or a crash inside the fold, resets the history as Clear display history does, counted too, rather than dropping every correct column after a mis-dated one. This supersedes the 2026-11-02 line above (#1635 Spectrum Q2): its reason still holds, but a backward column would misdraw a write-once history for up to ten minutes. #1758 Q A; Yan, 2026-11-14. (Superseded 2026-11-14 by #1784 S2, below: the guard is reduced to its debug assertion.)
- The bfloat16 rounding of stored powers goes with the column store it halved (#1505): a slab averages the analyzer's f32 power. A bucket whose mean sits on a dB step can draw one step lighter or darker. #1758

## 2026-11-14 (drift audit, #1784)

- A gap in the heatmap's columns always draws as silence, however short: the one-slab hold, which repeated the previous slab across a gap of one or two slab widths (an analyzer refill, a host pause), is gone, as its frame-jitter case went with the sample-grid FFT (4fb12e57). At the fresh History duration a seam shows as one 256 ms stripe of silence. #1784 S1; Yan, 2026-11-14.
- The spectrogram's dating guard is its debug assertion alone, as the stakes record asks of a case no producer writes: release builds no longer drop a backward column, reset the history on one or on a crash in the fold, or count either on the overlay. If a dating bug ever shipped, debug tests fail; in release a column dated backward averages into the open slab, and one dated far ahead erases the history and draws blank until Clear display history or until real time reaches that date. This supersedes the 2026-11-14 guard line above (#1758 Q A) and restores the 2026-11-02 line (#1635 Spectrum Q2). #1784 S2; Yan, 2026-11-14.
- A frame reads the analysis lag from the settings rather than from a lag carried with each batch of columns, so for a frame or two after a switch to a shorter Frequency resolution the strip's newest edge can stop short of the now-line. #1784 S3; Yan, 2026-11-14.

## 2026-11-18 (maintainability audit, #1861)

Yan took the recommendations made after the audit was checked against the code ("go with all recommendations", 2026-11-18).

- The spectrogram surfaces move from `spectrum.rs` into `spectrogram.rs`, so the dependency runs one way. The Tape heatmap's memo is built only keyed on a Tape revision count bumped when slabs settle, since Tape replaces slabs mid-history and the live check assumes an append-only pyramid. #1861 R4 (Built in #1892: the revision advances on every write to a copy, `put` and `clear`, not when slabs settle, since the copy the pane draws is written only by the sync and never settled.)
