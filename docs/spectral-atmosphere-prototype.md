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
- **Cloud texture** folds and varies the heatmap's surrounding light.
- **Note glow** adds ribbon bloom without changing the lattice's bloom setting.

The detailed heatmap keeps its bucket footprint and palette lookup.
A quarter-resolution image of the same geometry supplies two cascaded separable Gaussian filters,
whose light is composited around the detailed core.
Each pass uses 17 half-step taps;
the wide filter reads the softened close image to fill the gaps that sparse taps leave around narrow ridges.
Quintic gradient noise supplies two centered domain folds and three scales of billows and wisps.
Those folds displace the surrounding light in the pane's time and pitch directions,
while the detailed heatmap stays at its measured coordinates.
Warped light fades smoothly at texture boundaries.
Clouds remain inside the available audio-history strip in this prototype;
they do not extend into an unwritten startup region or a stale-data gap.
Filtering uses linear float textures;
the final screen blend preserves highlight headroom.
The cloud texture follows audio time and absolute pitch,
so paused history does not animate and scrolling does not move the texture independently of the sound.
Audio time is rebased over a matching 4096-second noise period before conversion to GPU floats.
Every noise octave uses an integer multiplier to keep that period intact through the nested folds.
The full history-window length chooses a temporal scale in powers of two,
so a short view still shows a handful of broad billows.
Changing that window can change the texture's scale;
new columns arriving and scrolling through a fixed window do not change its phase.

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
