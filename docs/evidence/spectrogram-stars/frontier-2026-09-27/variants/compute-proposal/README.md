# Bounded compute Stars halo experiment

Status: source proposal only; no compile, validation, dispatch or GPU measurement.
Repository was read only. Generated compute.wgsl is a snapshot of the research worktree's production shader on 2026-09-27; regenerate if production changes.

Three compute variants share the same 8x8 pixel groups and one dispatch with five Z layers: gather baseline, shared raw records, and shared decoded records.
The fourth control remains the shipping five fragment passes.

## What stays equal

- Pixel positions are integer output coordinates + 0.5, then the existing
  fs_star_halo transforms in the same f32 order.
- Every pixel evaluates its same 3x3 nominal-cell neighborhood, same radial
  response, same nine-add order, same per-star saturation and life fade.
- The output remains five RGBA16Float images; native cores and slice composition
  are untouched. Every in-bounds (x,y,z) is written exactly once, so no atomics,
  additive blending, accumulation order change or clear pass is needed.
- Full and half modes only differ by existing cloud.star_halo_size/output size.
  Do not combine this initial comparison with per-layer resolution overrides.

## What changes

The shared variants identify the rectangle of nominal cells covered by each screen group, add its one-cell neighbor border, then cooperatively read each atlas record once. At most 256 records are held. A larger rectangle falls back to the identical gather for that entire group. No group-per-star-cell dispatch,
dynamic allocation, cross-group synchronization or persistent cache exists.

Raw records use 4 KiB per group. Decoded records use 8 KiB: f32 center and inverse sigma/fade, f32 RGB and u32 active flag, 32 bytes per record. This removes repeated color unpack/divide and shape unpack as well as atlas reads. If both arrays survive pipeline specialization their combined size is still 12 KiB.

Decoded-record sharing might actually change compiler rounding of color divide or inhibit/enable contraction; validate its pixels separately. Reducing atlas instructions may merely replace hardware texture-cache hits with shared loads,
barriers and lower occupancy, so the raw-record variant is an essential control.
Compute storage writes may also lose tile-renderer advantages. A faster compute gather would be independently useful even if both shared variants lose.

## Minimal wiring and controls

Run make_compute_shader.py with the repository path. It copies the existing response mechanically into record and decoded-record helpers; generated source does not manually reimplement Gaussian, fringe, taper or saturation expressions.
integration.rs supplies the test-only constructor/dispatch and wiring locations.

Each timing case must own independent Targets and color history. Choose its mode before allocating, keep it fixed, and assert compute presence for compute cases.
Do not mutate the mode after Targets allocation unless it is an allocation key.
Only real compute output needs STORAGE_BINDING; scratch arrays and render controls should retain original usage. Avoid changing production corpus/layout.

Retain the existing source-begin -> dependent final-composite-end interval.
This compute pass has timestamp_writes=None and consumes no additional queries.
The dependency chain is source -> light -> star atlas -> compute halo -> final composition; the source/full interval remains meaningful. Optional new inner brackets need a dedicated pair for the single compute pass, never repeated writes into the old five-layer pass slots.

First compare fragment vs compute gather visually at full and half resolutions.
Then compare raw and decoded sharing against compute gather. Include dimensions not divisible by 8, narrow/wide panes, large cells that fit one nominal cell per group, fine cells that span many, and a fine/extreme-aspect case that actually exceeds the 256-record limit. Assert a fallback witness, not merely its branch.
Use a CPU count based on actual per-frame uniforms for fallback/reuse statistics rather than atomics in the timed shader. At least report cached-group fraction and loaded records/output pixel by layer.

Run the existing independent native reference at full resolution and preserve its f32 source. Compare silence, fading life, saturated overlap, fringe zero/max,
defocus max, jitter endpoints, fractional origins/scales and drift wraps.
Inspect max/mean pixel errors plus crops/motion. Compute vs fragment may round differently even when the math and sample positions are the same.

Bound the first benchmark to fragment/gather/raw/decoded at 1080p and 4K for resolution 0.5 and 1.0, with source compilation, warmup, interleaved ordering and a duplicate fragment A/A control. If there is no benefit beyond noise,
stop rather than adding compact binning, indirect dispatch or more caches.
