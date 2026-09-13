# Spectral atmosphere prototype

The Spectral pane can draw the blue-green audio field as softly diffused clouds,
with the purple-yellow note ribbons standing above it on their existing soft shadows.
Display → Analyzer → Atmosphere (prototype) controls the treatment.
It opens enabled;
disabling it restores the previous heatmap,
analyzer,
grid and ribbon-bloom paths.

- **Diffusion** blends fine spectrogram detail into a soft intensity field,
  while retaining strong pitch centers.
  At 0% the original detailed heatmap returns.
- **Cloud spread** sets their reach relative to the pane size.
- **Cloud texture** adds gentle density variations to the unified field.
- **Analyzer glow** controls the aura around the live spectrum.
- **Note glow** adds ribbon bloom without changing the lattice's bloom setting.

The measured heatmap and its diffused body share one intensity field and one palette lookup.
A quarter-resolution scalar image of the measured geometry supplies two cascaded separable Gaussian filters.
It holds display levels before coloring,
with no RGB or gamma conversion in the source or filters.
Each pass uses 17 half-step taps;
the wide filter reads the softened close image to fill the gaps that sparse taps leave around narrow ridges.
The broad filter reaches five times as far as the close filter,
so nearby pitch energy pools into a diffuse body.
The soft field mixes 75% close and 25% wide diffusion.
Diffusion blends the measured level toward this field;
detail retention rises smoothly between levels 0.45 and 0.95,
leaving full-bright centers intact while reducing low and medium grain.
The lattice's smooth value-noise recipe supplies one shared density across the pane,
with broad shapes and a softer detail layer.
That density only attenuates the combined intensity;
it never displaces it,
adds a tint or illuminates silence.
Its influence fades out at full brightness and as Diffusion approaches zero.
The soft level and texture density are baked at quarter resolution into the now-free source texture,
then bilinearly sampled beside the full-resolution measured level.
Shaping cost follows the reduced image size,
and this final bake adds no texture allocation.
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
The cloud medium uses aspect-correct pane coordinates,
like the lattice:
the spectrogram's intensity illuminates it at the active pitches as history moves through it.
Pitch zoom,
axis orientation and history-window changes do not reseed or rescale the medium.
It is stationary during playback and pause;
there is no independent cloud animation.

The analyzer uses the original sample positions for its shaded body and colored rim.
Only its surrounding aura is smoothed.
Both audio layers still use the spectral palette;
note ribbons still use the pitch palette and the existing shadow atlas.
The prototype reduces grid and now-line contrast without reducing label contrast.
The live editor and offline renderer share all these drawing paths.

Each spectrogram pane owns four reduced textures,
allocated by their pixel dimensions and evicted with the pane's existing lifetime.
Palette,
zoom,
time and appearance changes refresh source pixels and uniforms without reallocating the textures.
Grid uploads retain their existing generation/shape key and dirty-slab updates.
Empty or disabled frames never composite a retained cloud image.
The production Metal catalog explicitly constructs the atmospheric pipelines,
even though their runtime allocation remains lazy.

The `spectrum.atmosphere` section defaults missing fields individually and normalizes on load.
The new `diffusion` field defaults to 70% for existing appearances and recorded takes.
The existing `glow` field now controls only the analyzer,
and keeps its saved value.
No stored palette,
audio analysis or lattice setting is rewritten.
