# Spectral texture levels

Watercolor at 0% Random brightness displaces the scalar spectrogram picture without adding exposure,
lighting or pigment.
This is the A2/B2 choice from issue #1027:
sample the displaced levels,
then apply Contours and the selected palette.
It preserves the palette and the sampled levels,
rather than moving already-colored RGB pixels.
Interpolating levels before coloring retains the stepped look;
interpolating finished colors would soften those steps and can mix colors outside the gradient's path.

Softness and Spread still build the source picture using the existing density-weighted filter.
Watercolor reads that same combined field and keeps its overlapping globs,
lookup feathering and bleed,
and the Layers blend between coarse and fine sampled levels.
Texture mix blends original and displaced levels before the shared Contours and palette lookup.
Since #1042 there is no Cloud pixel size:
cloud sampling is fixed at 0.5 pt,
which is native on 1x and 2x displays,
so the displaced scalar field is reduced before that lookup only where a point spans more than two device pixels.
Tile geometry and the Watercolor tile rotation are unchanged.

At zero Refraction with Color pickup and Color release also zero (and Watercolor Random brightness at 0%) the renderer takes the ordinary texture-off path,
so the picture is byte-identical and no texture work is needed.
Zero Refraction alone does not reach that path at the shipped defaults:
Color pickup and Color release are on by default and keep temporal color memory running regardless of Refraction (see [color memory](spectrogram-color-memory.md)).
Refraction over a constant source cannot invent a pattern;
half-float intermediate storage and filtering can round the output by one channel byte.
Contour strength,
Contour levels and Contour edge softness remain the quantization controls and now affect the refracted picture even at full Texture mix.

Scale relief and Edge pooling are removed because they only changed lighting and pigment.
Their old saved keys are ignored;
other saved appearance settings remain readable.
Existing projects will look different with a texture enabled because the old tone adjustments are gone and Contours now applies.

Watercolor’s Random brightness varies globs after the palette lookup and color memory,
using a stable signed draw blended across their feathered edges and fine layer.
One linear-light gain preserves hue;
brightening and dimming share the same highlight headroom so clipping cannot bias the expected RGB average.
A finite view fluctuates around that average rather than being normalized every frame.
The slider defaults to 0%,
works without refraction,
and scales with Texture mix.
It uses spare geometry-tile channels and does not rebake the tile or reset held color.
