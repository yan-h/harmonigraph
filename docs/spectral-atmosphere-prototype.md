# Spectral atmosphere prototype

The Spectral pane can draw the blue-green audio field as close halos and broad clouds,
with the purple-yellow note ribbons standing above it on their existing soft shadows.
Display → Analyzer → Atmosphere (prototype) controls the treatment.
It opens enabled;
disabling it restores the previous heatmap,
analyzer,
grid and ribbon-bloom paths.

- **Spectral light** controls the heatmap clouds and analyzer aura.
- **Cloud spread** sets their reach relative to the pane size.
- **Cloud texture** controls how strongly the shared cloudy medium absorbs spectral light.
- **Note glow** adds ribbon bloom without changing the lattice's bloom setting.

The detailed heatmap keeps its bucket footprint and palette lookup.
A quarter-resolution image of the same geometry supplies two cascaded separable Gaussian filters,
whose light is composited around the detailed core.
Each pass uses 17 half-step taps;
the wide filter reads the softened close image to fill the gaps that sparse taps leave around narrow ridges.
The broad filter reaches five times as far as the close filter,
so nearby pitch energy pools into a diffuse field of colored light.
The lattice's smooth value-noise recipe supplies one shared density across the pane,
with broad shapes and a softer detail layer.
That density only attenuates the combined light;
it never displaces it,
adds a tint or illuminates silence.
The texture control blends this attenuation directly,
so its default leaves gentle variations instead of high-contrast curls.
The display-space material is baked at quarter resolution into the now-free source texture,
then bilinearly sampled beside the full-resolution heatmap core.
Shaping cost follows the reduced image size,
and this final bake adds no texture allocation.
Clouds remain inside the available audio-history strip in this prototype;
they do not extend into an unwritten startup region or a stale-data gap.
Filtering uses linear float textures;
the final screen blend preserves highlight headroom.
The cloud medium uses aspect-correct pane coordinates,
like the lattice:
the spectrogram's volume colors illuminate it at the active pitches as history moves through it.
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

The new `spectrum.atmosphere` section defaults missing fields individually and normalizes on load.
Existing appearances acquire the enabled prototype defaults;
no stored palette,
audio analysis or lattice setting is rewritten.
Recorded appearances missing this section also acquire these defaults when rendered with this build.
