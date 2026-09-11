# Settings guide

Open **Display** for picture and interface settings.
**Tuning** sets intervals and temperaments;
under CLAP it also configures adaptive tuning and each connected Tune instance.
**Video** records takes, frames the exported Lattice and Analyzer, chooses the spectrogram timeline and render trigger, and starts or reruns export.

| Display page | What you will find |
| --- | --- |
| Lattice | Camera and seventh layers; note-layer sizes and fade; audio ring; MIDI octave ring; melody/bass marks; labels and idle crosses. |
| Analyzer | View orientation and frequency range; audio input, resolution and smoothing; spectrum levels and response; shared history; MIDI ribbon appearance. |
| Colors | MIDI note colors by pitch and audio colors by level, with separate ranges and previews. |
| Lighting | Bloom on MIDI notes across all three picture panes; lattice glow; shadows for lattice shapes, lattice text, analyzer/Spiral notes, and analyzer/Spiral labels. |
| System | Lattice render resolution, frame limit and performance overlay; interface scale, tab-bar visibility and layout reset. |

**History duration** sets the live MIDI-ribbon and spectrogram window, and the Video page's **Scrolling** render uses the same span.
**Whole video** stretches that scrolling history to the render length, up to 10 minutes;
**Playhead** shows the full recorded spectrogram with a moving playhead and requires recorded audio.
**Spectrum level range** controls curve height and lattice audio-ring levels;
**Level color range** controls audio colors independently.
With the audio ring set to **Octave levels**, **Pitch tolerance** controls how far neighboring frequencies can light a slice;
in **Spectrum** mode, **Pitch span** sets the displayed range.
**Note match tolerance** on Tuning controls which MIDI pitches match lattice nodes, the Notes pane's node column, and the Analyzer's off-lattice band.

## Reading and entering values

- **%** measures a proportion or a position between named endpoints.
  Brightness runs from black at 0% to white at 100%, using perceptual lightness.
  Lattice gaps, cross dimensions, glow reach and lattice shadow width use the node radius as their reference.
  Held-note extension uses the spectrum region's depth.
- **×** measures a multiplier: label sizes, depth-layer size, bloom and glow gain.
  The tooltip states the reference that 1× multiplies.
- **ms** measures short response and fade times;
  **s** measures longer spans such as history duration and adaptive silence reset.
- **¢** measures cents, with 100 cents per semitone;
  **st** measures semitones, **Hz/kHz** frequency, and **dB** level.
- **°** measures camera angles.
  Analyzer and Spiral shadow widths use screen points (**pt**), independent of pitch zoom.
- MIDI note numbers set pitch centers and color endpoints;
  **px** sets video size, **fps** labels the frame-rate limit, **dB/oct** sets spectrum tilt, and adaptive delay is measured in host buffers.
- Layer, octave and buffer counts remain whole numbers;
  **Stop at bar** accepts fractional bars.
- The glow falloff curve keeps its signed shape value and a preview:
  zero is linear, positive fades early, and negative fades late.

Drag a numeric bar to change its value, or double-click to type.
Entry uses the displayed units:
`25%` or `25` means 25 percent on a percentage bar, and `250 ms` or `250` means 250 milliseconds on a timing bar.
Multi-handle bars instead reset on double-click;
hover text describes what each handle moves.
