# Central sequencing WIP architecture checkpoint

This is an incomplete #616 implementation checkpoint for the requested architecture reassessment.
It is not completion of the central sequencing stage or clearance to merge.
The committed base is GUI input `36cbf6276efe94742bd4d31532beaac3e63deac3`, on `codex/616-gui-parameter-input` (draft PR #681).
The implementation remains on `codex/616-central-sequencing`.
Further feature work is paused pending that reassessment.

## Implemented surface

Production Tune processing uses fixed D512, central complete-input cohort ordering, an intentionally artificial sequential correction, copied configuration bindings, and same-FIFO cohort commit markers.
The artificial correction depends on preceding prospective corrections;
it is not the #621 musical policy.
DIRECT retains zero adaptive correction.

Source output uses a generation/BUSY gate and a compact velocity-prefix/onset/initial-tuning group with actual accepted-prefix accounting.
Capture status and cancellation controls have a separate repair cell from ordinary ingress.
Retained captures, Life identities, plans and output history have distinct retirement conditions.

The Hub has a test-triggered fence, lifetime inventory, copied-status, factual-context rebuild, replay and resume path.
It replays only the finite old bound/frozen originals through its saved horizon, preserves accepted pitches and copied configurations, and assigns fresh decisions and emission generations to retained unsounded requests.
Later unbound originals remain on the normal input cursor.
Automatic activation of this recovery path is **not wired**.

The final bounded edit preserves original-On identity across inventory and cancellation ordering.
Unbound cancellation tombstones remain until their permitted Original is consumed, and bound terminal plans remain while a replay reader owns their incomplete copied status.
The Plan prefix/reader flags share one byte to retain the full future configuration budget.
Life retains the immutable On serial without increasing its backing slot by storing shift presence in an existing flags byte.

## Evidence and limits

The log names below are relative to `target/adaptive-sequencing/` in this worktree.
They are local receipts, not CI artifacts.
Earlier receipts describe the source snapshot when each command ran, not every subsequent edit.

| Receipt | Executed result and reach |
| --- | --- |
| `checkpoint-guarded-cancellation.log` | Current formatted checkpoint: the same three-scenario test passes with `--features nice-plug/assert_process_allocs`, 0.14 s; compile 35.17 s. |
| `cancellation-inventory-order.log` | Latest bounded edit: 1 passing test with three scenarios, 0.14 s; compile 26.36 s. Real Retained inventory before/after the bounded Source cancellation seam, bound cancellation after copied status, no canceled On, actual Life-index reuse, registry zero. This does not prove correlated transport Stop. |
| `hub-replay-first.log` | 1 pass, 0.06 s. Test-triggered real Hub recovery preserves an accepted note, renews an unsounded binding with its original configuration, preserves a 20-sample duration and admits later input. |
| `production-recovery-first.log` | The strengthened replay test also checks the later input uses the newer configuration. The run subsequently failed because the new stale-Off fixture consumed a Hub-owned status response; following tests aborted from the poisoned test lock. The failing receipt is retained. |
| `stale-off-binding.log` | Corrected fixture: 1 pass, 0.04 s. Stored and delayed old Off-mode assignments cannot emit after generation reopen; registry zero. |
| `joined-recovery.log` | 1 pass, 0.05 s before the final stronger setup assertion. Active recovery completes through joined Source/Hub owners with 65 same-key retained lifetimes and registry zero. The current fixture additionally asserts the fixed inventory total is 65 before joining; that strengthening still needs a rerun. |
| `partial-retirement.log` | 1 pass, 0.03 s. Accepted partial onset retains actual pitch/release debt and no completed Frozen capture label strands the retired owner. |
| `production-bootstrap.log` | Earlier 13-test pass includes canonical timing permutations, dense 64-note cohort delivery, full capture-window progress and GUI input classification. |
| `sixteen-256-delivery.log` | Earlier 16-source/256-voice completion proof, with measured extra delay. This is explicitly not D512 throughput qualification. |

The receipt commands above do not explicitly enable `nice-plug/assert_process_allocs`, so their success alone is not an allocation-guard receipt.
The checkpoint additionally passed the current targeted cancellation test with that feature explicitly enabled, retaining its separate result in `checkpoint-guarded-cancellation.log`.
The full production group, full workspace CI and final memory ledger have not been rerun on this checkpoint.
No manual Bitwig, destination-instrument or offline-host validation is claimed.

## Known incomplete paths

- Automatic missed-deadline, actual-divergence, configuration, membership and Stop recovery activation is not connected.
- A rejected On with no accepted prefix still enters blanket output-fault cancellation; retained-request recovery for that case remains unfinished.
- Correlated transport run/Stop handling, complete Off/Reset/rejoin and history behavior remain incomplete or unqualified.
- DIRECT baseline/loss repair does not yet update the new factual companion cache.
- Full-window recovery, partial wildcard consumers, in-flight acceptance races, repeated invalidation and resume backpressure require reaching fixtures.
- Cancellation currently transfers one disposition through the repair cell at a time; the old documented 2048-round retirement bound has not been re-proved for the expanded protocol. Batching and a revised bound were planned but are not implemented.
- Remaining real 8192/8193, 1025 and 257-capacity and release-backpressure cases are not completed.
- The final runtime/future memory ledger is outstanding. The last independent future subtotal was 150,640,891 bytes before the new Hub recovery owner, factual-cache metadata and subsequent Source/identity changes. Existing slot-size assertions remain, but they do not replace that ledger.
- Unused fields and unfinished paths still produce compiler warnings, so this checkpoint is not expected to satisfy the CI warnings-as-errors gate without further work.

## Saved-state and handoff notes

Canonical note deltas and take records add `partial_output` with the existing container defaults providing false for absent data.
This records an accepted onset whose initial tuning was rejected.
No compatibility alias, migration or persisted-version change was introduced.
No golden-image change is intended by this checkpoint.

Both release packages must be built after the checkpoint commit before handoff:

```sh
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 RUSTC_WRAPPER='' cargo build --offline --release -p harmonigraph-plugin -p harmonigraph-offline -j2
./load-plugin.sh --tag
```

The wrapper bypass uses the previously observed sccache permission failure and does not restart the shared cache daemon.
The shared DAW/plugin and offline-renderer slots remain untouched.
The draft PR remains open, draft and not merged.
