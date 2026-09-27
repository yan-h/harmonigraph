# Scratch fused star memory and bake

`wiring.patch` applies to the current research worktree and adds only scratch test behavior.
`fused.wgsl` is appended to the selected complete shader by the research source hook.
The patch adds `half-fused` and `full-fused` timing cases;
the existing image harness recognizes their resolution prefixes.
No production WGSL or persistent state changes.

The optional MRT pipeline reuses `tile_pipeline` and the existing explicit layouts.
Location 0 is RGBA32Uint (packed stars), location 1 RGBA32Float (updated memory).
Those attachments use 32 bytes/sample altogether.
Both must have identical allocation dimensions and logical extent;
the method falls back to the original passes when its prerequisites are absent.
The initial experiment keeps memory enabled.
Disabling memory falls back to the ordinary bake and never invokes the fused entry point.

`Targets::update` remains the only owner of the current ping-pong index,
previous slices/life, memory-valid flag and response coefficients.
The pass attaches `memory.views[memory.index]` and binds `memory.groups[memory.index]`, which reads `history[1-index]` and a scratch atlas.
It must not bind `memory.star_groups[memory.index]`, which reads the attached current history.
The caller skips the old remember and bake only after the fused method returns true.
No independent updates, resets, history copies or migrations are introduced.

The shader calculates life/hash/center/light/rank once,
uses the same periodic identity test and `remembered` formula,
then feeds that held color directly into the original 10-bit packing.
A silent star still writes updated memory but a zero atlas entry.
Padding outside the live slices writes zeros to both outputs.
Life turnover and invalid memory select current color as before.
Sizes, center bits and inverse-sigma/fade packing retain the original arithmetic.

## Concrete risks and verification

- Removing the RGBA32Float store/load removes a compiler boundary.
  All variables are still f32 and all targets keep the same formats,
  but Metal can schedule/fuse arithmetic differently,
  so exact 10-bit color equivalence is not guaranteed by the source alone.
  Compare final full-resolution outputs after multiple shared warmup frames,
  including a life rollover, silence after nonzero light, and a history reset.
  If image differences appear, inspect retained color and packed atlas separately before accepting them.
- RGBA32Uint plus RGBA32Float MRT is exactly 32 bytes/sample.
  Pipeline validation must confirm this device supports the combined attachments;
  do not silently substitute smaller precision targets.
- Fused cases use their own CallbackResources,
  so the optional pipeline is created only for a fused source and history remains independent.
  Do not switch a populated resource cache between ordinary and fused shader sources.
- Both targets currently have the same Stars extent by construction.
  The guard returns false if that changes;
  a resulting tie must not be presented as successful fusion without confirming the MRT pass ran.
- Extra stage timestamps distort scheduling and overlap.
  In diagnostic mode `bake_ms` represents the combined MRT pass and `memory_ms` is blank.
  Compare uninstrumented whole source-to-paint intervals;
  never sum stage durations or infer exclusive preparation cost.

Suggested bounded run: half-a, half-b, half-fused, full-fused at 1080p and 4K,
then a full-a/full-b batch if a repeatable whole-frame improvement is visible.
The full-fused case in the first batch is a quality-frontier point,
not an equal-resolution comparison against the half controls.

The patch passed `git apply --check` when created.
It has not been applied, compiled, or executed on a GPU by the proposal author.
