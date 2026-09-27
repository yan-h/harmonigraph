# Three grouped halo arrays (unapplied research follow-up)

Requires the five-separate-texture prototype already applied. No repository edit,
build, shader validation or GPU work performed here. Regenerate grouped-halos.patch with `python3 make_patch.py /path/to/harmonigraph` if surrounding code changes.

Added cases, all at base star_halo_resolution=1:

- full-grouped-v100_100_100_100_100: all-native grouping overhead control.
- full-grouped-v50_50_50_50_50: uniform-half equivalence/control.
- full-grouped-v50_50_80_100_50: accepted P2.
- full-grouped-v50_50_100_100_60: accepted P3.
- full-grouped-v50_50_100_100_100: F.
- full-grouped-v25_25_50_50_50: low2.

Every case owns independent resources after activate(). Five-separate-texture,
max-array/subrectangle, uniform-array, compute and other controls remain intact.
The grouping is intentionally a research setting captured through case-specific allocation, not a production cache key or persisted control.

## Mapping and safety

Groups are assigned by the first occurrence of each exact factor's f32 bits.
This is independent of rounded extent: the 1x1 scratch targets preserve the same three-group mapping even though all physical scratch widths/heights coincide.
More than three distinct factors fails before resource creation.

The same grouped_halo_plan supplies:

- The unique factors and layer counts for array allocation.
- Each original slice's group index and local array layer for its D2 render view.
- Representatives for whole-array sampled views in every bind group.
- WGSL constant maps for the composite's uniform three-way sampling switch.

For P2, slice groups are [0,0,1,2,0] and local layers [0,1,0,0,2]. Noncontiguous slice 4 shares the half-resolution array with 0/1 but is still composited last.
The five original render passes, per-slice shader instance indices, native cores,
far split and source/full timestamp bracket remain unchanged.

StarHalos gains no fields. Its existing five render views retain their underlying textures. At bind-group creation, representative render views produce temporary whole-array views; create_bind_group retains those resources before the temporary views drop. Missing groups alias group zero in unused fixed binding slots.

The existing `(array_view, render_views)` argument distinguishes real from scratch halos for every cloud bind group, including color-memory variants. The halo write group passes scratch render views, so all three derived sampling arrays are scratch. It never binds an array containing its actual output layer for reading.
Each render attachment remains a single-layer D2 view, so clearing one layer does not clear the other layers in that group.

Only a tiny 1x1x5 array remains at the original unused binding10. All meaningful halo pixels live in one, two or three actual-sized arrays. Actual extent and underlying layer-count assertions witness that the intended groups were allocated.
All writes use each original slice's ceil(base_size*factor) extent; reads use normalized pt/cloud.size and the group's genuine ClampToEdge sampler.

## Resource cost and bounded comparison

Three extra D2Array bindings at 11..13 bring sampled textures to twelve across group0+1 (nine original plus three), versus fourteen for five separate textures.
The two existing samplers are shared; no binding-array feature is requested.
The layout asserts the twelve-texture device limit. A production cleanup could remove the unused original array binding for eleven total, or specialize the number of bindings, but neither is needed to measure this control.

Compare grouped-native vs separate-native vs native-array first, then grouped all-half vs separate-all-half vs uniform half. This separates the mapping/switch cost from savings from smaller attachments. Compare grouped P2/P3 against their five-texture versions, both pixels and timings. Byte identity is not guaranteed by an unchanged mathematical sample position; compiler/filter rounding remains a measurement. Five passes and unchanged query-slot ownership keep timing fair.

The group plan and constant maps add modest host/source-generation complexity but bound the GPU resource footprint. That is a plausible shipping tradeoff if the accepted P2/P3 pictures retain their measured advantage. Measurements from the five-texture version alone cannot establish this grouped version's speed.
