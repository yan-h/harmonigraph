# Spectral atmosphere prototype

The Spectral pane can draw the blue-green audio field as softly diffused clouds,
with the purple-yellow note ribbons standing above it on their existing soft shadows.
Display → Analyzer → Softness and glow has three independent amounts.
Each can be set to zero without switching off the others.

- **Diffusion** removes fine spectrogram detail at every brightness.
  At 0% the original detailed heatmap returns;
  at 100% only the softened field remains.
  Its smoothing radius is fixed relative to the pane size.
  There is no procedural cloud texture or spread control.
- **Analyzer softness** blends the live analyzer from a flat fill into translucent shading
  and a soft colored halo.
  At 0% only the flat fill remains;
  at 100% the full treatment is applied.
- **Note glow** adds ribbon bloom without changing the lattice's bloom setting.

**Outline opacity** independently controls the analyzer's white contour under Audio analysis.
It keeps quiet frequencies visible while their fill retains the palette's dark colors.
At 0% the outline disappears.

The measured heatmap and its diffused body share one intensity field and one palette lookup.
A quarter-resolution scalar image of the measured geometry supplies two cascaded separable Gaussian filters.
It holds display levels before coloring,
with no RGB or gamma conversion in the source or filters.
The source averages its pitch footprint and takes four stratified time samples per reduced pixel,
reducing aliasing as fine temporal detail scrolls across reduced pixels.
Each pass uses 17 half-step taps;
the wide filter reads the softened close image to fill the gaps that sparse taps leave around narrow ridges.
The broad filter reaches five times as far as the close filter,
so nearby pitch energy pools into a diffuse body.
The soft field mixes 75% close and 25% wide diffusion.
Diffusion fades the raw contribution as `(1 - diffusion)²`,
so the default 10% leaves 81% raw detail and 100% leaves none.
Bright peaks are softened along with the rest of the field;
the close filter preserves distinct pitch bands without restoring their original grain.
The close and wide levels are combined at quarter resolution into the now-free source texture,
then bilinearly sampled beside the full-resolution measured level.
This keeps the final pass to one filtered read per pixel without another texture allocation.
The unified level is colored once through the existing palette,
interpolating between its samples.
Its first half-slice joins true black continuously,
including with an edited palette whose dark end is nonblack.
Only the final output performs the target's gamma or linear color conversion.
The soft field is baked and drawn across the whole spectrogram region,
so its Gaussian tails can fade into unwritten history or a stale-data gap.
The measured heatmap and the blur source keep the original history mesh.
The backdrop uses a zero measured level and the same diffusion and palette as recorded history,
then the opaque measured mesh replaces its own pixels with the unified field.
The analyzer divider and pane clip still bound this backdrop,
including when the axes turn or the divider moves.

The analyzer uses the original sample positions for its shaded body,
with one continuous mesh across the entire Softness range.
Only its surrounding aura is smoothed.
The white outline follows the measured edge;
its width and opacity are independent of Softness.
The stored zero-power level is excluded before tilt can lift it into a visible contour.
An entirely silent analyzer draws no outline.
Both audio layers still use the spectral palette;
note ribbons still use the pitch palette and the existing shadow atlas.
The grid and now-line keep their reduced contrast independently of these controls.
The live editor and offline renderer share all these drawing paths.

Each spectrogram pane owns four reduced textures,
allocated by their pixel dimensions and evicted with the pane's existing lifetime.
Palette,
zoom,
time and appearance changes refresh source pixels and uniforms without reallocating the textures.
Grid uploads retain their existing generation/shape key and dirty-slab updates.
Retained filter bindings follow grid and palette allocation changes even while diffusion is disabled,
so re-enabling after a resize or display-mode change reads current audio.
Empty frames and zero diffusion never composite a retained filtered image.
The production Metal catalog explicitly constructs the atmospheric pipelines,
even though their runtime allocation remains lazy.

The `spectrum.atmosphere` section defaults missing fields individually and normalizes on load.
Diffusion defaults to 10%,
analyzer softness to 50% and note glow to 50%.
Saved diffusion and note-glow values are retained.
The former `enabled`,
`glow`,
`spread` and `texture` fields are removed and ignored on load;
previously disabled appearances now use their stored diffusion and note-glow amounts,
and missing analyzer softness defaults to 50%.
The `spectrum.keyline` field retains saved outline opacity and defaults to 30% when missing.
The experimental `spectrum.analyzer_min_brightness` field is discarded;
appearances saved during that experiment restore the default outline opacity.
No compatibility shim or version bump is needed for discarded struct fields.
No stored palette,
audio analysis or lattice setting is rewritten.
