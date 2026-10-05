# Settings guide

The Settings column's tabs run from the pictures to the editor:
**Tuning** sets intervals and temperaments;
under CLAP it also configures adaptive tuning and each connected Tune instance.
**Lattice** and **Analyzer** hold everything their pictures draw, down to light and shadow.
**Mappings** holds the color tables and note intensity mappings every picture shares.
**Video** records takes, frames the exported Lattice and Analyzer, chooses the spectrogram timeline and render trigger, and starts or reruns export.
**System** holds rendering cost and the editor's own interface.
**Console** is the editor's log.

| Tab | What you will find |
| --- | --- |
| Tuning | Lattice tuning and temperaments, note matching, retuning engine and saved maps, adaptive tuning and connected tuning sources. |
| Lattice | **View**: camera, seventh layers. Independent sections for **Note layers**, **Octave layout**, **Note animation**, **Note labels**, **Audio ring**, **Idle lattice**, **Note bloom**, **Background glow**, **Glow pattern**, **Glow material**, and **Shadows**. |
| Analyzer | **Spectrogram**: pitch/time softness, level contours, Watercolor, Scales or Stars texture. **MIDI ribbons**: width, held-note extension, note names and bloom. **View**: dock, spectrum edge, shared frequency range, axis label scale and history, spectrum outline and backdrop. **Analysis**: audio input, frequency resolution and averaging, level mapping and tilt, live response. **Spiral** bloom. **Shadows** for Analyzer/Spiral notes and labels. |
| Mappings | MIDI note colors by pitch and audio colors by level, with separate ranges and previews. **MIDI note intensity** maps velocity, gain, pressure and timbre to opacity or thickness. |
| Video | Record take, output frame and size, history mode, re-rendering, and an interactive composition preview. |
| System | **Editor and exports**: lattice resolution and spectrogram time sampling. **Editor only**: frame limit and performance overlay; interface scale, interface lightness, tint and accent colors, tab-bar visibility and layout reset. |
| Console | The log, the held-note count and Clear. When this build refuses a project's saved editor settings, the editor opens on this tab with the reason. |

Settings opens on Tuning in a fresh workspace.
Tabs that do not fit the column move, from the right, into a trailing overflow menu.
Every section heading folds its section, and folds are remembered.
A section whose feature can be switched off (Spectrogram, MIDI ribbons) carries the switch in front of its name;
off, the section is its heading alone, dimmed and with no fold arrow.
Right-clicking the Lattice or Analyzer picture offers a link to its settings tab.
Each tab keeps a single owner for its controls;
shared settings name their scope in the help text.

## Finding a control

Lattice note geometry, animation, labels and light effects fold independently.
**Note layers** sits beside **Octave layout**;
**Audio ring** keeps restoration instructions when its width is zero.
**Octaves** divides the ring into 2–11 equal slices;
**Center pitch** sets the pitch at the top of every ring.
**Label size** also controls cross thickness.

Analyzer textures read in decision order:
choose **Style**, set **Texture mix**, then adjust **Motion**, **Appearance** and **Color response**.
For Stars, **Stars resolution** comes first among its controls in both pictures,
after Texture mix in the spectrogram and Material amount in the lattice,
at 75% by default.
**Color pickup** follows brighter sound;
**Color release** retains color as sound fades.
Both use seconds, with zero responding immediately.

Renamed sections start unfolded when opening older projects;
visual values and their saved keys are unchanged.
Detail folds such as **Map editing** reopen closed with the editor.

## Combined controls

Related settings can be edited directly in diagrams.
Drag a round handle,
or focus it and use the arrow keys;
Shift makes keyboard adjustments finer.
A fixed-width picture sits beside compact sliders with labels and value readouts.
Very narrow panes stack the values below the picture.
The sliders remain independently editable:
drag a bar or double-click it to type in the displayed units.

| Control | What the handles change |
| --- | --- |
| Ring threshold | The upper handle sets the opening threshold; the lower sets hysteresis. The white marker shows the effective closing threshold, including the small floor that prevents silent rings from staying latched. A zero opening threshold deliberately shows silent rings. |
| Drift | Point the vector along the direction of travel and drag outward for speed. Stars use a direction compass and retain their independent depth-dependent speed profile. |
| Note fade | Move the endpoint horizontally for duration and the midpoint vertically for curve shape. Duration remains host-automatable. |
| Cabinet depth axis | Point the endpoint along the depth axis; its distance from the origin sets the depth-step scale. |
| Softness | Move the bounding corner of the blur footprint: horizontally for time, vertically for pitch. |
| Response | The rising and falling curves have independent time handles. Zero is immediate; otherwise each handle marks one time constant, at 63% risen or 37% remaining. |
| Star size, spacing and speed | Move the far and near endpoints vertically, or the midpoint to redistribute change across depth. Size and spacing use a logarithmic vertical scale; speed is linear. |
| Shadow profile | Move the upper-right corner for width and darkness. Contour shadows have a curve handle; Blur shadows expose caster spread and show schematic extent because their actual profile depends on the caster. |
| Interface tint and accent | Click the swatch button for a hue/amount picker using the interface's own color model. |
| Contour levels | Drag the integer slider from 2 to 16, or double-click to type; the band preview shows the count. |
| Spectrum edge | Click one of four buttons forming a compact rectangle: Left and Right span both rows, with Top and Bottom stacked between them. |

These controls edit the existing saved values.
Saved keys are unchanged.
Contour levels default to 16 and are capped there;
older values above 16 are clamped when loaded.

## Condensed appearance controls

**Pitch softness** and **Time softness** set the spectrogram's single Gaussian blur.
The former Wide blur mix is fixed at its fresh value of zero;
the extra wide blur passes are removed.
Increase the softness axes for a broader field.
The balance of a close core and a separate broad haze is no longer independently adjustable.

**Pigment reach** replaces Pickup width and Pickup softness in Lattice → Glow material.
It grows the source band and its feather together,
keeping the captured profile's proportions.
The default is 275% of the node radius,
and zero disables both dark and colored pickup.
Dark pickup and Color pickup remain independent.

**Pattern contrast** at zero turns the glow pattern off;
there is no separate None/Clouds choice.
**Stagger spread** at zero starts every slice simultaneously;
Slice order chooses Circular, Bidirectional or Random stagger for positive spread.

Saved Wide blur mix and pickup width/softness keys are ignored;
old pickup settings start at the new default reach.
The retired pattern selector is ignored too,
so a previously bypassed pattern uses its saved contrast:
set contrast to zero to turn it off again.
Saved appearances using the removed Simultaneous order cannot parse;
the editor reports the refusal and loads defaults,
and offline export reports the parse error.
Other orders retain their spread and motion settings.

## Note bloom

**Note bloom** is one shared control in **Lattice → Note bloom** and **Analyzer → MIDI ribbons**.
Changing either slider changes both pictures.
It runs from 0 (off) to 2×,
with 1× as the reference halo.
The Spiral keeps its separate **Spiral bloom** control.
Bloom is independent of the note intensity mappings.

## MIDI note intensity

On **Mappings → MIDI note intensity**,
below **Audio level colors**,
the **Opacity** and **Thickness** groups each hold their base and incoming mappings.
Use **Add mapping** to assign a source;
each source can affect several targets with an independent weight in each group.
The **Delete** button removes only that mapping;
adding it again starts at weight 1.
Every target starts at its base and adds the weighted contributions routed to it.
**Opacity base** applies even with every mapping off.
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
and thickness between zero and **Thickness max** times its reference width.
An opacity base of 1 leaves no room for positive additions;
lower it to see velocity or pressure brighten the note.
Note-release fading still applies,
and the lattice's background node glow follows Opacity as the ink does,
shining as strongly as the node's most opaque slice,
but not Thickness.

The colored bands sit inside each standard base slider,
behind its label and value,
and share its value scale.
The bands have square left ends and rounded right ends.
Their colors match the weight slider fills;
each band shows that source's possible reach from the base.
The ranges keep the same colors when the pointer passes over them or a weight slider.
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
Hide tab bars is saved with the editor workspace and never in an appearance or take.
Tab takes priority over focus traversal except while editing text.

## Shared settings and dependencies

| Setting | Scope |
| --- | --- |
| Frequency range | Analyzer and Spiral views; does not limit which frequencies are analyzed. |
| Audio input, Frequency resolution, Spectrum averaging | All audio views, including spectrogram history. They do not alter pass-through sound. |
| Spectrum level range | Analyzer height and lattice audio-ring levels. Audio colors, including the Spiral's, have a separate Level color range. |
| Live attack/release | Live Analyzer, Spiral and lattice audio rings. Spectrogram history keeps unsmoothed measurements. |
| Ring attack/release | Additional audio-ring smoothing after the shared live response. |
| History duration | MIDI ribbons and spectrogram; also Video history → Scrolling. Fit video (max 10 min) expands both histories to the render length, up to 10 minutes. |
| Octave layout | MIDI ring, audio ring and melody/bass marks. The layer stack controls their widths and visibility. |
| Note fade | MIDI slices, marks and labels, plus audio-ring visibility. Background glow has its own response. |
| Motion and starting pose | MIDI slices and marks ease smoothly from the selected starting offset and scale. |
| Audio level colors | Shared palette; lattice rings substitute gray at Silent slice brightness for its quiet endpoint. |
| Glow pattern, material and breathing | Require nonzero Background glow reach and gain. Pattern varies the glow; material reshapes the result. Each effect retains its own settings when bypassed. |
| Lattice resolution, Spectrogram time step | Rendering quality/cost in the editor, preview and exports. Video output dimensions are separate. |
| Editor frame limit, interface scale, Hide tab bars | Editor only; exports run at 60 fps. |

Fresh projects use **Balanced** frequency resolution (8192 samples); saved Fast selections remain Fast.
Fast trades pitch precision for response time,
and its wider bins can lose low audio-ring fundamental wedges while their harmonics still light the pitch class.
The cutoff depends on sample rate and the material.
Fresh **Contour strength** is 0%, so textures start smooth.
**Silent slice brightness** lives in Note layers and colours silent MIDI octave slices on sounding or fading nodes,
plus the audio ring’s quiet endpoint.
Idle lattice positions use the separate label/cross brightness.

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
and connected **Tuning sources**.
**Pass through** leaves incoming note pitches unchanged while the lattice's display tuning still applies.
**Tuning** puts the **Meantone** switch beside **Major third** and **Marvel** beside **Harmonic seventh**.
A linked interval shows its calculated value with a ↔ indicator;
in narrow panes its switch wraps below the interval.
Meantone makes syntonic-comma equivalents share a pitch and name without syntonic marks.
Marvel makes the harmonic seventh share the augmented sixth’s pitch and spelling (`A♯−2` from C with Marvel alone,
plain `A♯` with both links).
Typing,
dragging,
presets and Learn recognize temperaments automatically when their interval relationship is within 0.5¢,
then impose the relationship exactly.
Editing a linked interval independently recognizes or releases its link;
editing its generators keeps it following them.
Switching a link off keeps the currently displayed interval and stays off until a relevant tuning entry or sufficiently informative Learn result.
Learn leaves relationships without enough evidence unchanged and preserves target intervals while retuning.
The third and seventh parameter ranges include every interval their links can derive,
so switching off a link does not clamp the pitch to an older slider limit.
There is no separate Auto setting.
Saved links and recorded configurations are restored as saved,
without running recognition again.
**Keyboard** and **Context** are expandable parts of Adaptive tuning.
In Lattice Map mode,
**Saved map** stays visible above the **Map editing** fold,
which holds audition setup, offsets, assignments and map management.
While auditioning,
the active-mode notice and **Return to arrangement** stay beside **Saved map**,
even when **Map editing** is closed.
**Tuning sources** appears only when connected tuning sources are available.
Host map-offset parameters explicitly name whole generator steps rather than tuning intervals.

Video keeps **Finish recording** and **Stop at bar** with Record take.
Finishing a take starts rendering;
**Re-render take** applies the current appearance and video settings to the last take.
**Output size (px)** shows the actual width and height for the chosen aspect ratio.
Drag the lattice to an edge of the preview to arrange the pictures.
Shift-drag either picture to navigate,
and scroll or pinch to zoom.
**Fit video (max 10 min)** changes exported history;
the preview continues to use live **History duration**.

## Specialist controls to reconsider

This is a code-based assessment of complexity and overlapping concepts,
not evidence of how often a setting is used.

| Candidate | Recommendation and tradeoff |
| --- | --- |
| Slice order, stagger, starting offset/scale | Best candidate for motion presets with an expandable custom section. Keep the current freedom until preferred presets are known. |
| Four shadow groups, each with shape/width/darkness/falloff | Prefer basic width/darkness and expandable shape/falloff if the page still feels too long. Falloff is independent of width for Contour shadows. |
| Ring hysteresis and extra attack/release | Keep available as advanced response controls: hysteresis prevents threshold flicker, while smoothing changes level motion. |
| Glow pattern, material and breathing | Pattern and material have their own folds; breathing stays with glow response. Consider presets if only a few combinations prove useful. |
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
**Shadow spread** appears for Blur shadows.
It expands the shadow caster before blurring without changing the visible ink.
The percentage is a fraction of the resolved Shadow width:
100% expands the caster by one Shadow width on each side.
It defaults to 0% in new and existing saved settings;
a Shadow width of zero still removes the shadow.

Lattice text shadows use local darkness for both Contour and Blur.
They attenuate bright slices and finished bloom only inside their footprint,
with foreground ink covering shadows behind it.
Darkness defaults to 100%;
saved projects retain their existing setting.

Drag a numeric bar to change its value, or double-click to type.
Entry uses the displayed units:
`25%` or `25` means 25 percent on a percentage bar, and `250 ms` or `250` means 250 milliseconds on a timing bar.
Multi-handle bars instead reset on double-click;
hover text describes what each handle moves.
