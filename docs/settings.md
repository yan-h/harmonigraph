# Settings guide

The Settings column's tabs run from the pictures to the editor:
**Tuning** sets intervals and temperaments;
under CLAP it also configures adaptive tuning and each connected Tune instance.
**Lattice** and **Analyzer** hold everything their pictures draw, down to light and shadow.
**Mappings** holds the color tables and note intensity mappings every picture shares.
**Video** records takes, frames the exported Lattice and Analyzer, chooses the spectrogram timeline and render trigger, and starts or reruns export.
**System** holds rendering cost and the editor's own interface.

| Tab | What you will find |
| --- | --- |
| Lattice | **View**: seventh layers, camera. **Notes**: note-layer sizes and gap, note animation, labels, shared octave layout, audio ring. **Idle lattice**: idle brightness and crosses. **Light**: bloom and background glow, background glow texture and breathing, and shadows for lattice shapes and text. |
| Analyzer | **Spectrogram**: pitch/time softness and wide blur mix, level contours, Mosaic, Watercolor or Stars texture. **MIDI ribbons**: width, opacity, held-note extension, note names and bloom. **View**: dock, spectrum edge, shared frequency range, axis label scale and history, spectrum outline and backdrop. **Analysis**: audio input, frequency resolution and averaging, level mapping and tilt, live response. **Spiral** bloom. **Shadows** for Analyzer/Spiral notes and labels. |
| Mappings | MIDI note colors by pitch and audio colors by level, with separate ranges and previews. **Note intensity** maps velocity, gain, pressure and timbre to opacity, bloom or thickness. |
| System | Lattice resolution and spectrogram time sampling; editor frame limit and performance overlay; interface scale, skin and skin lightness, tab-bar visibility and layout reset. |

Settings opens on Tuning in a fresh workspace.
Tabs that do not fit the column move, from the right, into a trailing overflow menu.
Every section heading folds its section, and folds are remembered.
A section whose feature can be switched off (Spectrogram, MIDI ribbons) carries the switch in front of its name;
off, the section is its heading alone, dimmed and with no fold arrow.
Right-clicking the Lattice or Analyzer picture offers a link to its settings tab.
Each tab keeps a single owner for its controls;
shared settings name their scope in the help text.

## Note intensity

On **Mappings → Note intensity**,
below **Audio level colors**,
the **Opacity**, **Thickness**, and **Bloom** groups each hold their base and incoming mappings.
Use **Add mapping** to assign a source;
each source can affect several targets with an independent weight in each group.
The **Delete** button removes only that mapping;
adding it again starts at weight 1.
Every target starts at its base and adds the weighted contributions routed to it.
**Opacity base** and **Bloom base** apply even with every mapping off.
**Thickness base** applies too,
in multiples of the pane's reference width:
**Ribbon width** in the Analyzer and the MIDI layer width in the Lattice.
Its default of 1× starts at that reference width;
0.5× starts at half of it.
A thickness contribution of 1 adds one such width;
a zero-width layer remains hidden.

| Source | Contribution before multiplying by its weight |
| --- | --- |
| Velocity | 0 to 1; full velocity adds one whole weight. |
| Pressure | 0 to 1; unpressed adds nothing and full pressure adds one whole weight. |
| Gain | Linear gain: silence adds nothing, unity (0 dB) adds one whole weight, and gain 2 (about +6 dB) adds twice the weight. Cuts add less but never subtract. |
| Timbre | −1 to +1: minimum subtracts one whole weight, center (0.5) adds nothing, and maximum adds one whole weight. |

Only timbre can reduce a target below its base.
The contributions are summed before the final limits:
opacity stays between 0 and 1,
bloom between 0 and 2,
and thickness between zero and **Thickness max** times its reference width.
An opacity base of 1 leaves no room for positive additions;
lower it to see velocity or pressure brighten the note.
Note-release fading still applies,
and the lattice's separate background node glow does not follow these mappings.

The rounded bands sit inside each base slider,
behind its label and value,
and share its value scale.
A light vertical marker shows the base.
Their colors match the weight slider fills;
each band shows that source's possible reach from the base.
Hover a band or edit its weight to highlight it.
Striped ends mark clipping at zero or the target ceiling.
Gain's solid band ends at unity gain;
the dashed continuation shows that boosts can extend it farther.
These indicators show configured possibilities,
not a live note reading.
Lowering **Thickness max** also clamps **Thickness base** to that ceiling.

Old one-target-per-source routes and weights are ignored on load,
so those mappings must be added again.
Base values are retained.
New mappings use these additions:
velocity no longer subtracts,
gain no longer uses a signed dB offset,
and timbre has twice its former range.
The retired **Gain range** setting is ignored on load.
Saved opacity bases above 1 are clamped to 1,
and saved bases now apply even when opacity has no route.
Saved projects without **Thickness base** load it at 1×,
preserving their pane widths.

## Layout and folding

The editor has three sections: Lattice, Analyzer, and Settings.
Analyzer and Spiral are alternative tabs in the same section.
The Analyzer tab's **Dock** setting places the Analyzer **Right** of or **Below** the Lattice;
each arrangement remembers its own divider sizes.
Settings stays to the right of the pictures.

Each section folds independently using its header arrow.
Click anywhere on its labelled rail to reopen it.
Folding a side-by-side section narrows the window;
folding a stacked picture shortens it while preserving the other picture's height.
Settings follows the available height.
When both stacked pictures are folded,
their rails move beside Settings for a compact settings-only layout.
Host minimum sizes or refused resize requests can make the open panes share the available space.

Drag the separators to resize visible sections.
Tabs and sections cannot be dragged into arbitrary arrangements.
**System → Reset layout** restores the default arrangement and unfolds everything.

## Folding the Analyzer

The arrows at the outer ends of the Analyzer's spectrum and history regions fold them independently.
Click a labelled rail to restore its region at its remembered size.
The spectrogram fold includes the piano roll and its note labels;
the Analyzer's tab-bar arrow still folds the whole pane and remembers both region states.

With the spectrum on the left or right,
folding shrinks the editor window while keeping the other panes and the surviving region at their current sizes.
Host size limits can force the open panes to share the remaining space.
Top/bottom orientations and an Analyzer placed below the Lattice fold within the existing pane size.
Restoring both regions brings back their previous split at the original spectrum position.
Changing the spectrum position while folded does not lose the width owed to the editor window.
Reset layout returns the space held by pane and region folds together.
These folds are saved with the editor layout and do not change the Video preview or exported composition.
Frameless mode hides their controls;
press Tab to show them again.

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
| Note fade | MIDI slices, marks and labels, plus audio-ring visibility. Background glow has its own response. |
| Motion and starting pose | MIDI slices and marks ease smoothly from the selected starting offset and scale. |
| Audio level colors | Shared palette; lattice rings substitute gray at Idle ring brightness for its quiet endpoint. |
| Background glow texture and breathing | Requires nonzero Background glow reach and gain. Its bypass preserves the effect settings. |
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
| Background glow texture and breathing | Keep together beneath their parent glow. Consider presets if only a few combinations are useful; the master bypass remains useful for comparison. |
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
  Lattice gaps, cross length, background glow reach and lattice shadow width use the node radius as their reference.
  Held-note extension uses the spectrum region's depth.
- **×** measures a multiplier: label sizes, depth-layer size, bloom and background glow gain.
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
- The background glow falloff curve keeps its signed shape value and a preview:
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
