# Settings guide

Open **Display** for picture and interface settings.
**Tuning** sets the musical intervals;
**Video** controls recording and export.

| Display page | What you will find |
| --- | --- |
| Lattice | Camera and seventh layers; note-layer sizes and fade; audio ring; MIDI octave ring; melody/bass marks and shimmer; labels and idle crosses. |
| Analyzer | View orientation and frequency range; audio input, resolution and smoothing; spectrum levels and response; shared history; MIDI ribbon appearance. |
| Colors | MIDI note colors by pitch and audio colors by level, with separate ranges and previews. |
| Lighting | Bloom across all pictures; lattice glow; shadows for lattice shapes, lattice text, analyzer/Spiral notes, and analyzer/Spiral labels. |
| System | Render resolution, frame limit and performance overlay; interface size, tab visibility and layout reset. |

**History duration** applies to both the piano roll and spectrogram.
**Spectrum level range** controls curve height and lattice audio-ring levels;
**Level color range** controls audio colors independently.
The audio ring's **Pitch tolerance** controls which nearby frequencies contribute to an octave level;
**Note match tolerance** on Tuning controls which MIDI pitches match lattice nodes.

## Reading and entering values

- **%** measures a proportion or a position between named endpoints.
  Brightness runs from black at 0% to white at 100%, using perceptual lightness.
  Lattice gaps, cross dimensions, glow reach and lattice shadow width use the node radius as their reference.
  Held-note extension uses the spectrum region's depth.
- **×** measures a multiplier: label sizes, depth-layer size, bloom, glow gain and shimmer contrast.
  The tooltip states the reference that 1× multiplies.
- **ms** measures short response and fade times;
  **s** measures history duration.
- **¢** measures cents, with 100 cents per semitone;
  **st** measures semitones, **Hz/kHz** frequency, and **dB** level.
- **°** measures camera angles;
  **steps** and **steps/s** measure shimmer spacing and movement on the lattice.
  Analyzer and Spiral shadow widths use screen points (**pt**), independent of pitch zoom.
- Counts remain whole numbers.
  The glow falloff curve keeps its signed shape value and a preview:
  zero is linear, positive fades early, and negative fades late.

Drag a numeric bar to change its value, or double-click to type.
Entry uses the displayed units:
`25%` or `25` means 25 percent on a percentage bar, and `250 ms` or `250` means 250 milliseconds on a timing bar.
Multi-handle bars instead reset on double-click;
hover text describes their handles and reset behavior.

The cleanup changes presentation and navigation without rescaling saved settings or recorded automation.
The new Lighting page is saved as a new page choice;
older builds cannot read a saved state with that page selected.
