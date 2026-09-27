# Actual separate halo textures (unapplied research patch)

No repository edits, build or GPU run performed. Regenerate against the current research checkout using `python3 make_patch.py /path/to/harmonigraph` if needed.

The five scratch cases are:

- full-separate-v100_100_100_100_100: all-native sampling/switch overhead control.
- full-separate-v50_50_50_50_50: actual half-sized textures, compare uniform half.
- full-separate-v50_50_50_100_100: existing D/full-mix4 vector.
- full-separate-v50_50_100_100_100: existing F/full-mix5 vector.
- full-separate-v50_50_80_100_50: an existing optimized three-resolution vector.

All cases use base star_halo_resolution=1.0. Any five-factor vector can be used for images; add its same name to CASES for a timed run. The existing factors() parser supplies both actual CPU texture extents and the shader's baked extents.

## What is isolated

For these names only, StarHalos allocates five RGBA16Float 2D textures at each layer's actual ceil(base_size*factor) extent. The original array binding is only a 1x1x5 dummy, not a hidden full-sized allocation. Its layers field now holds views of the separate real textures; the existing five render passes draw them.
Actual texture dimensions are asserted against the requested extent at draw.

The halo bake still computes pixel centers with cloud.size/research_halo_size(k).
The composite uses normalized pt/cloud.size directly with the texture's own ClampToEdge sampler. It has no maximum-array warp or subrectangle clamps.
A uniform switch on k chooses the fixed texture binding; there is one filtered read for that slice, not five reads/selects. Native core and depth order remain.

Extra texture bindings exist only for separate cases. All groups carry them,
including the history-dependent composite groups. The halo write group binds five scratch views, so no output is also declared as a sampled input in its pass.
Baseline groups/layouts retain the same entries, though constructor arrays are assembled through Vec to permit research extension. This construction is outside steady timing. Existing compute/fusion/u8 variants remain separate experiments.

## Why the allocation experiment matters

A scissor/viewport reduces shaded fragments, but LoadOp::Clear operates on the whole attachment. The old subrectangle proxy can still clear/store full-sized layers and retain their whole memory footprint. Five right-sized attachments therefore test costs that a shading-area regression cannot infer. Actual clear and store traffic remains hardware-dependent; do not assume it equals bytes.

Keep the five passes and existing stage query slots unchanged. First measure separate all-native vs native array and separate all-half vs uniform-half array;
these reveal the extra resource-selection/layout cost before interpreting mixed vectors. Compare pixels against the corresponding old subrectangle vector and native/uniform-half controls. A corrected sampling implementation can differ at rounding thresholds without an intended change in quality.

## Resource budget and engineering cost

The existing group1 layout declares eight sampled textures plus two samplers;
group0 adds one LUT texture. Five extra 2D textures bring the total to fourteen sampled textures and still two samplers. The pipeline constructor asserts at least fourteen sampled textures per shader stage. Binding slots in group1 become 0..15, sixteen entries, with no texture-binding-array feature required.

Fourteen textures is close to the common sixteen-texture floor and reserves little room for future effects. The extra groups/selection are worthwhile only if timings distinguish this layout from the simpler uniform array. Five fixed textures are an experimental control, not the preferred shipping architecture.

If right-sizing wins, two/three arrays grouping equal-resolution layers can use two/three added bindings (eleven/twelve total sampled textures with the current dummy retained), plus compile-time k->group/layer mapping and the same five attachment views/passes. Removing the unused original array binding could save one more slot in a production design. That design needs its own measured control:
the result of this five-texture switch cannot establish grouped-array overhead.
