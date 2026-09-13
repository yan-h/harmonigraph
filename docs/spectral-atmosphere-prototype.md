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
- **Cloud texture** varies the heatmap's surrounding light.
- **Note glow** adds ribbon bloom without changing the lattice's bloom setting.

The detailed heatmap keeps its bucket footprint and palette lookup.
A quarter-resolution image of the same geometry supplies two separable Gaussian filters,
whose light is composited around the detailed core.
Clouds remain inside the available audio-history strip in this prototype;
they do not extend into an unwritten startup region or a stale-data gap.
Filtering uses linear float textures;
the final screen blend preserves highlight headroom.
The cloud texture follows audio time and absolute pitch,
so paused history does not animate and scrolling does not move the texture independently of the sound.
Audio time is rebased over a matching 4096-second noise period before conversion to GPU floats.

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

The new `spectrum.atmosphere` section defaults missing fields individually and normalizes on load.
Existing appearances acquire the enabled prototype defaults;
no stored palette,
audio analysis or lattice setting is rewritten.
Recorded appearances missing this section also acquire these defaults when rendered with this build.
