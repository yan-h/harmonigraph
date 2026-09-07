# Selected spectrogram fixes (#654)

Only SG4A (#658), SG1 (#655), and SG2 (#656) are selected.
The other audit proposals remain deferred and are not follow-up work.
Base:
`927e2189` (main, including the tuning/replay integration in #709).
The active editor-startup branch was checked before implementation;
its changed files do not overlap the dedicated spectrogram or offline caller files.

One writer works sequentially in the Codex-managed `codex/654-spectrogram-fixes` worktree.
Three read-only subagents investigated the individual algorithms and fixture reach.
Each implementation commit is a separate Claude review target, followed by a review of the combined candidate.
The shared DAW slot is not changed.

## SG4A: warm upload staging

Aligned dirty slabs are queued directly from the immutable run.
An empty dirty list allocates no slab scratch;
generic unaligned bin counts still use one zero-padded scratch slab.
The full upload, callback packet, serial acknowledgement, ring mapping, and shaders are unchanged.

A temporary thread-local global allocator probe counted `alloc`, `alloc_zeroed`, and `realloc` requests inside the actual warm `CallbackTrait::prepare` grid-upload branch.
Counting was suspended around `queue.write_buffer`, excluding driver/native staging allocations.
Callback inputs and GPU resources were constructed before the measured warm branch.
The probe was run before and after the change, then removed.
These are allocator-requested bytes, not RSS or a prepare-time/FPS measurement.

| Bins | Dirty slabs | Before (requests × bytes) | After |
|---|---|---|---|
| 3828 | 0, 1, 2, 3 | 1 × 3828 | 0 |
| 3827 | 0 | 1 × 3828 | 0 |
| 3827 | 1, 2, 3 | 1 × 3828 | 1 × 3828 |
| 3829 | 0 | 1 × 3832 | 0 |
| 3829 | 1, 2, 3 | 1 × 3832 | 1 × 3832 |

[Before](sg4a-before.txt) and [after](sg4a-after.txt) contain the distinct observed rows, including the existing 256-bin fixture.
The committed `a_delta_upload_draws_what_a_full_upload_draws` fixture reaches these dirty-list sizes, reads across the full bin range, and compares persistent-resource frames to fresh full uploads through ring wrap, negative keys, generation changes, and capacity recreation.
Acknowledgement checks remain in the same sequence.

Commands:

```sh
cargo test -p harmonigraph-render spectrogram::tests:: -- --test-threads=1
cargo test -p harmonigraph-offline golden:: -- --test-threads=1
```

Validation at the SG4A checkpoint:
nine renderer spectrogram tests and all five existing spectral goldens passed.
No golden changed.

## SG1: bounded live gap recovery

The live owner chooses the retained key floor before appending or rebuilding.
The shared fold clips missing-slab expansion to that floor;
the batch/whole-song driver supplies no live bound.
At most one complete predecessor slab seeds a clipped one-slab hold or establishes a real black gap.
The original gap length still decides whether missing slabs hold or stay black.
All three vectors trim together, and admission releases capacity exceeding twice the retained budget plus its predecessor allowance.
Tiny vectors retain Rust's minimum allocation floor.

An excluded pre-gap target no longer overwrites the first retained black slab during partial-edge repair.
The incremental guard admits that excluded target at the retention floor, so resumed frames do not repeatedly rebuild from coarse history.
A larger retained budget still rebuilds when it reaches behind existing coverage.
No CPU storage protocol, callback protocol, cache owner, or shader changes are involved.

`live_gap_folds_bound_work_and_capacity_before_serving_the_view` exercises warm append, cold rebuild, and a changed rung at 12/180/600-second spans with 1/2/60/600/620/1,000,000-second gaps.
It checks each bounded fold before `view`, including vector capacities, and checks the final retained grid and three subsequent frames.
Because folding never shrinks capacity, these capacity checks also bound its intermediate allocation requests:
no more than `2 × (1032 + 1) × 3828 = 7,908,648` power bytes in the fixture, independent of the gap duration.
Short-gap output matches the unbounded batch reference;
long-gap slabs retain absolute timestamps and black contents.
A separate small-budget fixture verifies a clipped hold's complete predecessor MAX, reaching farther back after a budget increase, and release of deliberately oversized backing allocations.

The pane-level `resumed_slabs_fill_the_black_window_and_recover_gpu_uploads` reaches the real drawing function and checks callback presence on the first resumed occupied slab.
Its sequential GPU images compare delta and full uploads, including a rung rebuild and context reset, and assert full far-edge coverage and black gap pixels.
[Inspected resume sequence](sg1-resume.png):
warm (`false`) and cold (`true`) starts agree;
steps 1–2 resume at a 12-second span, step 3 changes to 24 seconds, and step 4 advances again.
The bright edge grows with new audio while the retained silent window stays black.
When production's 610-second history retention removes all pre-gap columns, a cold pane with only one new column retains the existing startup/no-callback behavior.

The capacity matrix intentionally uses direct history admission beyond 610 seconds to stress the fold independently;
the pane test separately reaches real history trimming via `push_history`.

SG1 checkpoint validation:
47 spectrogram tests passed (one profiling test ignored), and all five existing spectral goldens passed unchanged.
SG4A's independent Opus/medium review of `a024cb86` returned no findings.
