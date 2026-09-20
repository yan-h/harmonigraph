# Settings guide

Open **Display** for picture and interface settings.
**Tuning** sets intervals and temperaments;
under CLAP it also configures adaptive tuning and each connected Tune instance.
**Video** records takes, frames the exported Lattice and Analyzer, chooses the spectrogram timeline and render trigger, and starts or reruns export.

| Display page | What you will find |
| --- | --- |
| Lattice | Camera and seventh layers; note-layer sizes; shared octave layout; melody/bass marks; audio ring; note animation; labels and idle crosses. |
| Analyzer | Orientation and shared frequency range; live spectrum fill/outline; shared history duration and clear; MIDI ribbons, glow, extensions and labels. |
| Spectrogram | Heatmap visibility; pitch/time softness and wide blur mix; level contours; Mosaic or Watercolor texture. |
| Analysis | Audio input and frequency resolution; spectrum averaging; level mapping and tilt; shared live attack/release. |
| Colors | MIDI note colors by pitch and audio colors by level, with separate ranges and previews. |
| Lighting | Shared MIDI bloom; lattice glow, texture and breathing; shadows for lattice shapes, lattice text, Analyzer/Spiral notes, and Analyzer/Spiral labels. |
| System | Lattice resolution and spectrogram time sampling; editor frame limit and performance overlay; interface scale, tab-bar visibility and layout reset. |

Display opens on Lattice in a fresh workspace.
Each page keeps a single owner for its controls;
shared settings name their scope in the help text.
The two extra pages add navigation choices but remove the texture controls from the path to basic analysis and ribbon settings.

## Shared settings and dependencies

| Setting | Scope |
| --- | --- |
| Frequency range | Analyzer and Spiral views; does not limit which frequencies are analyzed. |
| Audio input, Frequency resolution, Spectrum averaging | All audio views, including spectrogram history. They do not alter pass-through sound. |
| Spectrum level range | Analyzer/Spiral height and lattice audio-ring levels. Audio colors have a separate Level color range. |
| Live attack/release | Live Analyzer, Spiral and lattice audio rings. Spectrogram history keeps unsmoothed measurements. |
| Ring attack/release | Additional audio-ring smoothing after the shared live response. |
| History duration | MIDI ribbons and spectrogram; also Video history → Scrolling. Whole video expands both histories to the render length, up to 10 minutes. |
| Octave layout | MIDI ring, audio ring and melody/bass marks. The layer stack controls their widths and visibility. |
| Note fade | MIDI slices, marks and labels, plus audio-ring visibility. Glow has its own response. |
| Motion easing and starting pose | MIDI slices and marks. Smooth eases to rest; Overshoot adds a small swell and overshoots displaced starting poses. |
| Audio level colors | Shared palette; lattice rings substitute gray at Idle ring brightness for its quiet endpoint. |
| Glow texture and breathing | Requires nonzero lattice Glow reach and Glow gain. Its bypass preserves the effect settings. |
| Lattice resolution, Spectrogram time step | Rendering quality/cost in the editor, preview and exports. Video output dimensions are separate. |
| Editor frame limit, interface scale | Editor only; exports run at 60 fps. |

With the audio ring set to **Octave levels**,
**Pitch tolerance** sets how far neighboring frequencies can light a slice;
in **Spectrum** mode,
**Pitch span** sets the displayed range.
**Note match tolerance** on Tuning controls which MIDI pitches match lattice nodes and the Analyzer's off-lattice band.
Adaptive **Same-note tolerance** decides which onsets count as one context note.
These are three independent tolerances.

Tuning separates lattice pitch/temperaments,
note matching,
output **Note retuning**,
and connected **Instances**.
**Pass through** leaves incoming note pitches unchanged while the lattice's display tuning still applies.
**Keyboard** and **Context** are expandable parts of Adaptive tuning.
Host map-offset parameters explicitly name whole generator steps rather than tuning intervals.

Video keeps **Finish recording** and **Stop at bar** with Record take.
Finishing a take starts rendering;
**Re-render take** applies the current appearance and video settings to the last take.
**Output size (px)** shows the actual width and height for the chosen aspect ratio.
The preview arranges the two pictures directly.

## Specialist controls to reconsider

This is a code-based assessment of complexity and overlapping concepts,
not evidence of how often a setting is used.
No creative control was removed in this pass.

| Candidate | Recommendation and tradeoff |
| --- | --- |
| Slice order, stagger, starting offset/scale | Best candidate for motion presets with an expandable custom section. Keep the current freedom until preferred presets are known. |
| Four shadow groups, each with shape/width/darkness/falloff | Prefer basic width/darkness and expandable shape/falloff if the page still feels too long. Falloff is independent of width for Contour shadows. |
| Wide blur mix | Candidate to replace with a fixed blend if comparisons show little practical value. It mixes close and five-times-wider blur; it is not redundant with either softness axis. |
| Outer octave scale/taper | Specialized layout refinements; could move behind an expandable group. Removing them would lose unequal end-octave layouts. |
| Ring hysteresis and extra attack/release | Keep available as advanced response controls: hysteresis prevents threshold flicker, while smoothing changes level motion. |
| Glow texture and breathing | Keep together beneath their parent glow. Consider presets if only a few combinations are useful; the master bypass remains useful for comparison. |
| Lattice resolution above 100% | Candidate for removing costly supersampling if visual comparison shows no useful improvement. Existing range is retained. |
| 144 fps preset and Frame breakdown | Candidates for simplifying the editor controls or moving diagnostics to Console. Neither is a video setting. |

The Spiral needs no separate settings page:
its frequency/level ranges,
analysis and colors are shared,
while pan and magnification are gestures.
Its remaining controls are drag to pan,
scroll/pinch to magnify,
and double-click to reset.

## Reading and entering values

- **%** measures a proportion or a position between named endpoints.
  Brightness runs from black at 0% to white at 100%, using perceptual lightness.
  Lattice gaps, cross dimensions, glow reach and lattice shadow width use the node radius as their reference.
  Held-note extension uses the spectrum region's depth.
- **×** measures a multiplier: label sizes, depth-layer size, bloom and glow gain.
  Spectrogram time step measures effect-sample spacing in multiples of a displayed history column.
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

**Shadow falloff** runs from −6 (early decay) through 0 (linear) to +6 (late decay).
Its preview shows the Contour shadow profile,
which reaches zero at one Shadow width and never bends into an S curve.
Blur shadows keep their Gaussian profile.

Drag a numeric bar to change its value, or double-click to type.
Entry uses the displayed units:
`25%` or `25` means 25 percent on a percentage bar, and `250 ms` or `250` means 250 milliseconds on a timing bar.
Multi-handle bars instead reset on double-click;
hover text describes what each handle moves.
