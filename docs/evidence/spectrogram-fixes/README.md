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
