# Raw shared compute group shapes

Unapplied research-only patch; no builds or GPU calls. Run make_patch.py against the current research checkout to regenerate before applying.

New timing cases: half-compute-raw-g4 (4x4), half-compute-raw-g16 (16x8).
Existing half-compute-raw remains the byte-identical 8x8 shader control.
The image harness also accepts full-compute-raw-g4 and full-compute-raw-g16.

One compute_group() result is captured at pipeline construction and retained in ResearchHaloCompute. It drives all four WGSL mutations (workgroup size,
pixel origin, pixel extent and cooperative loading stride) and both dispatch axes. Every mutation requires exactly one source match. A changed shader fails loudly. Draw never consults the currently active case, avoiding mismatch if the harness interleaves independently constructed resources.

The 256 raw-record bound and whole-group fallback remain unchanged. Every cached record has one writer among x*y lanes, followed by the original barrier before any reads. Partial edge groups still keep all lanes until after the barrier.
No zero initialization, extra shared array, timestamp query or format change.

Expected tradeoffs, not performance claims:

- 4x4: four times as many groups/barriers and more overlapping border loads,
  same fixed 4 KiB allocation per group, but smaller cell rectangles can avoid fallback for fine stars. Sixteen lanes also may underfill a 32-lane SIMD group.
- 16x8: half as many groups/barriers and less duplicated border work per output,
  while preserving horizontal pixel locality. More threads/registers per group and larger rectangles can reduce occupancy or trigger the 256-record fallback.
- 8x8: existing control. Keep identical resource formats and timing brackets.

Prioritize 16x8 for the suggested barrier-amortization gain; 4x4 is mainly a counterexample testing occupancy/fallback sensitivity. Neither necessarily fixes the compute storage-write cost that can lose to the tile renderer at 4K.

Run small irregular image dimensions (or fractional PPP) once to exercise both edge group shapes, then compare all three at half-resolution 1080p and 4K with paired A/A controls. Report fallback fraction by layer if it changes materially.
