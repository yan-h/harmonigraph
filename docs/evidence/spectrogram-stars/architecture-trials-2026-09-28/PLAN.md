# Stars architecture trials

User request: “please try all optimizations/architectural changes in order of how promising they are”.
Cost means visual difference from the current shader plus ongoing maintainability.
Current production reference is P3 at commit 7711b98d2c28c737b5a9c44dd298eb3d96efe589.
3+2 is a separate implementation stream;
include its known shader partition as a control rather than claiming ownership of that change.

## Ranked experiments

1. Four-neighbour direct complete response versus four-neighbour residual halos and native cores.
   Use identical shortened support (0.85 cells at default jitter), all five depths, unchanged star identity, paint and memory.
2. Fixed per-depth hybrids: direct shortened response except on a fixed mask of wide-halo depths.
   Screen wide masks 4, 8, 12, 16, 24 and 28 (bits numbered from far=0).
3. Whole depth-group reduced-resolution complete response: farthest three or all five at 75% and 50% dimensions.
4. Focused combination: farthest three with short glow at 75% and 50% dimensions.
   A native short-group control and confirmation matrix remain pending a quiet GPU window.
5. Small same-look optimizations: omit impossible neighbouring core subtraction and replay measured native complete response.

Already measured visual changes (four layers, P2) remain evidence references rather than being called newly discovered optimizations.
Already rejected sprites, shared blur, within-cell clipping and temporal caching receive no speculative port.

## Method

Scratch hooks live only in the owner-managed worktree.
Fixed variants share a release host binary and independently allocated GPU resources.
Baseline A/A controls are byte-identical shader sources.
Use real source-pass BEGIN to dependent composite END GPU timestamps.
Use balanced permutations for up to four cases, balanced forward/reverse rotations otherwise.
120 warmup rounds precede measurements.
Separate timing and capture/build work; retain process snapshots and all samples.
Timing take fixture is a 20-second recording-derived level image cycled one slab per frame, not the exact plugin analyzer mapping.
39.149967-second display history and production star defaults are used.
Synthetic and sparse confirmation distinguish workload dependence.
Actual appearance movies use the offline renderer, real .take input, a full 40-second lead-in, and the final six seconds at 768x432/24fps.
Their source state and pipeline are more faithful than the timing fixture's reconstructed light.
Include flat-input controls and native/enlarged stills.

No unselected experimental mode will become a persistent production setting.
Preserve source replay patches, scripts, hashes, raw timing data and conclusions in an evidence-only draft PR.
