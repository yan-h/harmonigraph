# Offline golden failure fixture audit

The four changed frames all exercise the current default Uniform Stars renderer.
They are not evidence of four unrelated shader routes changing.

- `a_tall_pane_zoomed_out_draws_the_frame_on_record` and `a_zoomed_in_pane_draws_the_frame_on_record` call `Shot::take`, which creates fresh `PictureState` and leaves `cloud_style` at the current default Stars.
- `mixed_spectral_shadows_draw_the_frame_on_record` also creates fresh `PictureState` and never changes its atmosphere style; its shadows and roll overlay the default Stars background.
- `the_starfield_draws_the_frame_on_record` explicitly selects Stars, which is already the default.
- Tall-pane and starfield committed PNG files are byte-identical: both SHA256 `065a640ba85dcb5197e6438e53e29de8ffb30cdbee0fee9909cf9ae891744867`.
- The watercolor fixture explicitly selects Watercolor and passed.
- These fixtures disable color pickup/release, so this golden change cannot be caused by history carry across profile changes.

The comments in `golden.rs` saying preceding frames exercise refracting scales describe old defaults and no longer identify their actual shader route.
The short-pane fixture also uses Stars but passed; sparse one-level differences need not affect every resolution.

`harmonigraph_golden::Gate` uses zero tolerance.
Its failure output rounds the mean to three decimals, so `mean0.000/255,max1/255` means a sparse nonzero difference, not contradictory statistics.
This is compatible with the bounded Uniform one-level drift already measured in the production parity captures.
It is not, on its own, causal proof or authorization to rebaseline.

The root agent's exact-baseline release/source-mode rerun, using the same fixtures and shader mode, is the appropriate bounded control.
If baseline passes and the new frames differ only at the reported sparse1LSB pixels, record them as changed Uniform Stars output after contact-sheet review.
If baseline also fails, compare baseline actual pixels to new actual pixels before deciding whether any rebaseline belongs to this change.
Keep source-vs-embedded mode identical in the comparison; shader assets initialize their mode once per process.

No repository edits, builds or GPU work were performed by this audit.
