# Spectral texture levels

Mosaic and Watercolor displace the scalar spectrogram picture without adding exposure,
lighting or pigment.
This is the A2/B2 choice from issue #1027:
sample the displaced levels,
then apply Contours and the selected palette.
It preserves the palette and the sampled levels,
rather than moving already-colored RGB pixels.
Interpolating levels before coloring retains the stepped look;
interpolating finished colors would soften those steps and can mix colors outside the gradient's path.

Softness and Spread still build the source picture using the existing density-weighted filter.
Both textures read that same combined field;
Mosaic no longer reads the wide field independently of Spread.
Mosaic keeps positive face refraction and negative gathering toward scale centers.
Watercolor keeps its overlapping globs,
lookup feathering and bleed,
and the Layers blend between coarse and fine sampled levels.
Texture mix blends original and displaced levels before the shared Contours and palette lookup.
Since #1042 there is no Cloud pixel size:
cloud sampling is fixed at 0.5 pt,
which is native on 1x and 2x displays,
so the displaced scalar field is reduced before that lookup only where a point spans more than two device pixels.
Tile geometry and the Watercolor tile rotation are unchanged.

At zero Refraction the renderer takes the ordinary texture-off path,
so the picture is byte-identical and no texture work is needed.
Refraction over a constant source cannot invent a pattern;
half-float intermediate storage and filtering can round the output by one channel byte.
Contour strength,
Contour levels and Contour edge softness remain the quantization controls and now affect the refracted picture even at full Texture mix.

Scale relief and Edge pooling are removed because they only changed lighting and pigment.
Their old saved keys are ignored;
other saved appearance settings remain readable.
Existing projects will look different with a texture enabled because the old tone adjustments are gone and Contours now applies.
