# Reduced halo bandwidth: RGBA8Unorm scratch experiment

Status: unapplied patch, no build/GPU validation. No repository changes.
Regenerate with `python3 make_patch.py /path/to/harmonigraph` if necessary.

Added timing/image cases: half-u8r4, full-u8r4, half-u8r9, full-u8r9.
The existing half/full f32 cases are the comparison controls.
Each case must create its own pipelines, Targets and memory after activate();
the research format choice is deliberately not a production allocation key.
Compute/fused cases are unchanged and cannot be combined with these suffixes.

The patch changes only the five-layer halo array and its fragment pipeline attachment from RGBA16Float (64 bits) to RGBA8Unorm (32 bits). It does not change the native far-layer intermediate, atlas, color memory, core or composition.
Writes divide all four channels by range, and sampled halo values multiply by range before joining the native core. Linear filtering therefore retains its meaning except for quantization and, at range 4, potential clipping. Never normalize RGB/coverage before storing: its native core has not joined yet.

Range 9 is safe: Gaussian<=1, full=min(Gaussian+fringe,1)*outer_fade<=1,
core>=0, halo=max(full-core,0)*life_fade<=1 per star. Nine stars sum<=9.
Packed palette RGB is in [0,1], so each summed RGB channel<=coverage<=9.
The same bound covers every current density/defocus/jitter/size setting.
Range 4 is an explicitly lossy candidate, not a proved-safe bound. The geometry does not forbid more than four nonzero contributions, including diagonal stars.
Any measured absence of clipping describes those fixtures, not the full range.

Main concern: range9 quantizes coverage in steps of 9/255≈0.0353; an isolated sample below ~0.0176 rounds to zero. Range4 steps are 4/255≈0.0157, with ~0.00784 half-step. RGB is independently quantized at the same scale: weak halo colors may change sharply when normalization follows the native-core join. This is a large precision reduction despite halving bandwidth. Bright cores do not protect dark, isolated halos or lifetime transitions from banding/temporal stepping.

Optional image-only diagnostics: half-u8r4-clip, full-u8r4-clip, and corresponding u8r9-clip names. They write a binary mask where any original unscaled halo channel exceeds the range, then render the sampled maximum over slices as red on black,
with star cores and far split bypassed. In the existing image harness cloud_depth is 1, so an all-black pane means no clipped halo samples were observed. Upsampling filters that mask, so this diagnoses presence/location, not an exact source-sample count or clipped-value magnitude. Do not use clip diagnostics for performance.

Bounded evaluation: first render baseline/r9/r4 and their amplified differences at full and half resolution over the real take and flat input; include a short drift/lifetime movie. If quality is visibly unacceptable, stop without a long timing campaign. Check r4 clipping masks at least at default and extreme controls.
If quality remains usable, run interleaved baseline/variant/A-A measurements,
report bandwidth gains separately from deliberate appearance change. A reduction in target bytes is certain; a speed gain is not, particularly if math dominates.
