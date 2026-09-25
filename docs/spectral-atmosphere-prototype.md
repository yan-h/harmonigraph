# Spectral atmosphere

This is a historical record of a control set that no longer exists.
PR #928 retired the Plain/Blur/Lava style selector described below,
replacing it with independent dials whose zero is off,
and PRs #888, #909, #913, #918 and #933 added, renamed and re-ranged the whole Cloud texture section this file predates.
`SpectralAtmosphere` now carries twenty-five persisted fields where the shape section below describes six.
Since #1083 there is no Display tab;
these controls live on the Analyzer settings tab.
Since #1117 there is no Analyzer softness:
the analyzer body is always the flat fill.
Since #1095 the saved outline opacity `spectrum.keyline` is `spectrum.keyline_lift`,
how far dark outline colors are brightened toward white.
The current inventory is the Analyzer and Spectrogram pages in `crates/harmonigraph-ui/src/panes/spectral/settings.rs` and `impl Default for SpectralAtmosphere` in `crates/harmonigraph-scene/src/atmosphere.rs`;
those are the source rather than a copy of it here.
The current [texture level contract](spectral-texture-levels.md) describes the shared displacement and color behavior.

The Spectral pane draws the measured audio field with independent spectrogram style,
analyzer softness and note glow controls under Display → Analyzer → Softness and glow.

- **Spectrogram style** selects Plain,
  Blur or Lava.
  Pitch softness in cents and time softness in milliseconds smooth all measured content equally;
  Spread combines close and wide smoothing,
  and Lava adds nested contours with adjustable edge softness.
  Both softness widths at zero preserve the unsmoothed field.
- **Analyzer softness** blends the live analyzer from a flat fill into translucent shading
  and a soft colored halo.
  At 0% only the flat fill remains;
  at 100% the full treatment is applied.
- **Note glow** adds ribbon bloom without changing the lattice's bloom setting.

**Outline opacity** independently controls the analyzer's white contour under Audio analysis.
It keeps quiet frequencies visible while their fill retains the palette's dark colors.
At 0% the outline disappears.

The spectrogram now averages linear power and uses the smoothed field directly in Blur and Lava.
A fixed brightness weighting before source reduction gives narrow bright content more influence,
and Lava fades contour strength in across its first band to preserve quiet detail.
The former Diffusion amount is removed.
See [spectrogram averaging and Lava](spectrogram-lava.md) for the scalar filtering,
allocation policy,
performance measurements and validation fixtures.

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

The `spectrum.atmosphere` section defaults missing fields individually and normalizes on load.
Lava is the default,
with 35-cent pitch softness,
120-ms time softness,
25% spread,
seven contours and 15% edge softness.
The old `diffusion` field is ignored on load;
new controls take their defaults.
Analyzer softness and note glow retain their independent saved values and default to 50%.
The `spectrum.keyline` field retains saved outline opacity and defaults to 30% when missing.
