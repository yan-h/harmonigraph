# Terminal fault checkpoint

This stage starts from `2f9fe9318491d946cd34d54b7e7f7eb786f0f0e9`.
The selected policy is in [the contracts](adaptive-tuning-contracts.md).
Actual storage exhaustion, required input/identity/clock evidence loss and rejected or partial host output inhibit new performance;
a never-enrolled independent Source stays local, while an enrolled or frozen prospective contributor conservatively closes the affected session.
Display/reporting loss, ordinary lateness, bounded work or ring backpressure, healthy Stop/Off and configuration waiting do not themselves begin this transaction.

A terminal transaction fences emission, inventories retained lifetimes, acknowledges exact cancellation dispositions and drains its captured factual output cut.
It then finishes with the gate CLOSED.
Physical release debt and the Reset latch remain independently owned;
rejected emergency retries cannot recreate the completed transaction.
Only explicit Reset followed by a valid complete-input callback with a live matching session clears inhibition.
A newly observed fault consumes that authorization even if its class matches the old latch;
re-reading previously published fault bits does not.

Actual producer destruction with retained wire state or release debt publishes owner-loss evidence to that Source's fault row before its ProducerJoined message can wait behind a full mailbox.
The predicate is sampled before fault handling can arm repairs.
An empty Source with unknown initial controllers, or an accepted Off waiting only for factual acknowledgement, does not satisfy it.
The Hub uses retained membership to classify scope and close existing emission gates;
a group that acquired its permit before the close still completes truthfully, while an unclaimed group cannot acquire a permit after the close.
Physical debt from the destroyed producer remains owned without fabricated termination.

The Hub retains request identities for the whole finite recovery reader.
Canceled unbound identities survive until their Original is consumed;
terminal cohort delivery is paid once before a full reply ring or identity hold can delay retirement.
A DONE Original with a legitimately retired Plan remains valid.
Abandoning a delivery generation clears both its unsent count and its decision interval in the same owned operation.
Otherwise Finish can wait for factual output while an old terminal Plan attempts to pay against an already cleared count.
Ordinary context rebuild retires the same interval when it abandons old deliveries; retained identities and actual output cuts remain separate.
A local terminal request arriving during ordinary recovery waits behind that reader without closing unrelated participants or discarding its pending continuation.
The failed nonmember's initial baseline is factually published and acknowledged after its terminal cut, without enrolling it or pinning healthy publication forever.

Live final Seal and Detach messages precede replaceable coverage-only Progress messages.
The final accepted output cut still publishes its continuous coverage once before Seal, allowing a full receiver output window to drain.
Otherwise continuously advancing valid coverage occupies the sole ordinary control cell each callback and prevents an otherwise settled Reset from completing.
Existing baseline and accepted-history ownership still precede the seal.

## Joined ownership work

`registry::service_retired` is an off-audio ownership pump, not a callback timing guarantee or a physical termination deadline.
Every round services every retained Source and Hub across the fixed registry.
The bound assumes joined incarnations and therefore no new host input or host output acceptance;
unknown physical debt remains retained.
Live peers may require another callback and no cleanup claim follows for them.

The selected ceiling is **1,149,345 outer rounds**.
It is deliberately conservative, and the pump normally stops much earlier.
Its terms are explicit in `registry.rs`:

| Debit | Rounds | Population/proof |
| --- | ---: | --- |
| Capture scans, revisits and acknowledgement | 688,128 | Per row `8192 * (2048/64 + 3*1024/64 + 4)`; rows have separate 64-entry parser/scan slices |
| Cancellation/control ownership | 172,032 | `4 * (8192 parents + 32768 children + 1024 ingress + 1024 replies)`; publication, crossed control, ACK and receiver retirement |
| Recovery | 11,040 | Four complete shared Prepare/inventory passes; local transactions' Source scans/chunk handshakes are summed across 16, with two ordinary and one global pass |
| Factual output | 270,336 | `2 * (16 * (4096 journal + 2*2048 receiver/transport + 128 emergency allowance) + 2048 DIRECT)`; one round per receipt and application, including expensive actual-voice lookup |
| Shared input-grant interference | 1,297 | `ceil((2402304 finite traffic units + 65536 repair allowance)/1904)` |
| Final baseline/control/attachment | 1,024 | `4 * 16 * 16` for retained handovers and their finite revisit/ACK tails |
| Released-reader Source rescan | 5,472 | `(16+3)*(8192+2*32768)/256`; parent visits plus completed child traversal and reclamation |
| Final quiet revisit | 16 | One complete 1024-entry ingress revolution at 64 visits per round |

Per-row latency is a maximum, not seventeen serial copies.
Each outer round services every Source and every Hub row;
shared grant denial is charged separately across all seventeen rows.
Routine visits use at most `17*64 + 16*64 + 16 = 2128` of the 4096 input-work grant.
If a row is denied its next maximum 65-unit operation, at least 1904 additional units belong to finite accepted/install/retire/control traffic.
The traffic allowance is `3 E + 2 N + 2*17*1024 = 2402304`, where `E=17*(8192+32768)` and `N=17*8192`, plus 65536 units for repair control.
Recovery and factual waits have their own debits.
Blocked recovery and joined-output-wait visits cost one unit;
the full token charge remains mandatory whenever retirement can actually walk its children, including newly applicable InputSettled proof.

Terminal Prepare scans only selected indexed Source rows.
Before Reset, disjoint local masks collectively cost one whole pool, a later session transaction costs one, and an active plus already queued ordinary continuation cost two.
The recovery debit is `4*(2*16*8192/256 + 4*16) + (16+3)*(3*8192/256 + 2*8192/64)`.
This counts the shared Hub grant and serial local transactions separately from parallel Source scans.

After the exact matching ProducerJoined output cut is received, a retired Hub drains that row's finite factual output before retiring its Captures.
That cut certifies no later output, independently of untransferred Capture originals and InputSettled.
The latter can be blocked behind a full input window and cannot establish this proof by itself.
An empty output window plus the exact joined cut avoids scanning the same output once per Capture;
this does not invent local Work completion or physical termination, and the Source retains those independent owners.
A live Source without this proof retains the existing output-reference scan.
The ceiling conservatively retains the scan term even though this joined path removes it from most all-joined drains.

Ending a recovery reader also restarts the existing cancellation-cut cursor for joined Sources.
Their off-audio pump does not run the live output scheduler's pending scan, so without this explicit revisit a locally completed Original can keep its Life reference forever after the reader unpins it.
The separate 5,472-round debit charges parents and two complete child walks across serial local and ordinary/global transaction populations; it does not reuse the inventory count/fill/Life-cleanup allowance.

Only actual cancellation-cursor/phase changes and output-reference scan decrements count as progress.
Starting a fresh blocked ingress rotation does not.
After actual ownership progress stops, at most 16 quiet rounds allow the current finite parser sweep to revisit an actionable owner;
a blocked peer cannot keep resetting that allowance.
This replaces the obsolete 1964/2048 assumption of 64 dispositions per round;
the actual repair protocol carries one independently acknowledged disposition at a time.

## Verification scope

The production factory fixtures cover crossed inventory/cancellation order, full ordinary replies with unpaid canceled delivery, DONE originals without Plans, partial output with persistent physical debt, repeated rejected emergencies, local fault scope both with and without an ordinary reader, and explicit Reset/fresh admission including a renewed same-class input fault.
The pressure fixture retains 4096 locally completed raw-clock parents, fills both input windows, retains factual output and 3072 unsounded same-ID lifetimes (48 inventory chunks), then faults through real input inspection and destroys both owners.
Its actual last registry drain exceeds 2048 rounds and ends with no retained registry owners.

Six practical 15-note cases preserve simultaneous five-note Source groups, initial tuning, expression at +10 and release at +20, with no terminal fault.
Measured A/B/C extra samples are:

| B onset offset | A,C,Hub,B order | B,A,C,Hub order |
| --- | --- | --- |
| 447 | 1 / 0 / 1 | 0 / 0 / 0 |
| 448 | 0 / 0 / 0 | 0 / 64 / 0 |
| 511 | 0 / 0 / 0 | 0 / 1 / 0 |

The extra A/C sample in the 15-note favorable 447 case is a further D512 miss.
These are functional allocation-guard receipts, not latency qualification.
Ordinary prospective late reconciliation remains a separate stage;
this branch must not classify these delays as terminal faults.

Ordinary nonhead publication pressure is tracked in [issue 686](https://github.com/yan-h/harmonigraph/issues/686).
The default uncalibrated DIRECT stall is tracked in [issue 685](https://github.com/yan-h/harmonigraph/issues/685).
The broader plugin suite is not green, and this checkpoint is not timing or DAW qualified.
Exact guarded commands and their results are recorded with the final handoff.

## Checkpoint receipts

All tests use `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 RUSTC_WRAPPER=''`, this worktree's target and `--offline`.
The production allocation guard is explicitly enabled with `--features nice-plug/assert_process_allocs`.
The guarded `production_` group passed all 30 tests after the owner-loss and delivery-generation review corrections.
The Capture ownership group passed all 10 tests, including real Pending/Birth reuse and mixed-generation captured targets.
The live retired-Hub fixture measured 52 callbacks to settle 3,584 accepted Original/output owners after Hub destruction;
it allows 64 callbacks and keeps the distinct all-joined pump bound above.
The earlier 33-callback loop was too short for the new terminal inventory handshake.
The 8,192-capture/8,064-cancellation Reset fixture also passes with one repair disposition at a time.
Consistent-prefix sequence-exhaustion fixtures pass in both mapped and sealed streams.
The velocity-prefix fixture verifies that a fault inhibits a new nonzero prefix while preserving an essential Off: a neutral prefix is accepted, the Off is rejected, and its retry needs no duplicate neutralization.

```sh
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 RUSTC_WRAPPER='' cargo test --offline -p harmonigraph-plugin --features nice-plug/assert_process_allocs production_ -j2 -- --nocapture --test-threads=2
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 RUSTC_WRAPPER='' cargo test --offline -p harmonigraph-plugin --features nice-plug/assert_process_allocs performance::tests::capture_tests:: -j2 -- --nocapture --test-threads=2
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 RUSTC_WRAPPER='' cargo test --offline -p harmonigraph-plugin --features nice-plug/assert_process_allocs measured_ordinary_storage_and_actual_factory_allocation_increments -j2 -- --nocapture --test-threads=2
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 RUSTC_WRAPPER='' cargo clippy --offline --workspace --all-targets -j2 -- -D warnings
cargo fmt --all
./.claude/semantic-breaks.py --check
```

Workspace Clippy, formatting and semantic Markdown checks pass.
The broader plugin run remains failed: ordinary credit/publication and controller-replay expectations require their own triage alongside the two issues above.
The targeted Stop group has five passes and four failures, recorded with an exact old-condition comparison in [issue 690](https://github.com/yan-h/harmonigraph/issues/690).
Both the actual-voice Stop condition and the preceding credit-based condition produce the same four failing assertions;
the ACK-only owner-loss correction does not cause them.
Neither the selected groups nor the guarded allocator measurement substitute for full CI, independent review or practical D512 qualification.

The review correction adds two reaching fixtures.
The owner-loss fixture destroys an enrolled Source with an accepted held note while a peer owns a bound but unclaimed onset, then observes terminal completion with the peer inhibited and the destroyed Source's physical debt still retained.
Its empty-source variant uses uninitialized controller state and a real healthy Stop, then verifies the peer's complete note, tuning, expression and Off and zero remaining registry owners.
Its acknowledgement-only variant accepts the Off before the Hub can acknowledge it, retaining one credit with zero actual voices, known-neutral CC64 and initially unknown CC66/69.
Retirement must preserve that absence of wire debt and let the peer finish;
the pre-correction Stop gate used the retained credit to manufacture controller resets and falsely reported owner loss.
Stop now keys fresh release debt on actual voices, held pedals, owed Offs or a nonzero velocity prefix.
The accounting fixture keeps a real assignment reply ring full, advances one Source continuously ahead of its Hub, and accepts an established Off at its actual D512 due callback.
It reaches ordinary Rebuild with one unpaid bound Plan and an unapplied factual cut, then a real input-inspection fault cancels that On and promotes terminal recovery.
The pre-correction run reached the service_plans assertion inside the actual allocation-guarded Hub callback; the correction retires the old delivery interval while Finish still waits for output.
Ordinary recovery activation remains outside this checkpoint's qualification.

## Memory ledger

The final isolated guarded allocator fixture measured an ordinary session at 106,125,144 bytes and four sessions at 424,500,576 bytes.
The earlier subtotal before the test-only Frozen owner marker was 106,125,128 bytes.
Source is 31,256 bytes, Row 31,128, Sequencer 5,808 (including its 3,936-byte Recovery), and SourceControl 31,248.
The additional factual-voice backing is 10,240 bytes and its free-index allocation is 512 bytes for 256 `u16` entries;
the printed free-capacity value is already bytes.
The 24-byte Vec metadata and Box metadata are included in Sequencer and are not added again.

Against the independently checked inventory projection 150,640,891, the future delta is 5,416 bytes:
17 Source headers add 680, 16 Rows add 256, Sequencer adds 3,968, and the new factual free-index backing adds 512.
The resulting specified future subtotal is 150,646,307 bytes, leaving 348,637 below 144 MiB.
The later test-only Frozen ownership marker adds 16 measured Hub bytes and no production bytes;
the final allocator rerun confirms those 16 bytes, and charging them conservatively gives 150,646,323 and 348,621 spare.

The physical ordinary increase 23,976 is 5,416 plus 10,240 factual-voice backing and 8,320 inventory-window growth.
The latter two remain within existing prepaid allocations and must not be charged twice.
The factual/prospective voice reservation retains both 65,536-byte halves.
The complete future inventory/cancellation allocation is 13,456 bytes:
6,784 for the window, 16 Arc header, 64 future configuration expansions from 64 to 128 bytes, and 64 manifest cells of 40 bytes.
It stays within the unchanged 16,384-byte prepaid manifest allocation.
SourceControl does not grow.

All 131,072 future Plan cells and all Source Life cells retain their 256-byte ceilings, including full 128-byte resolved configuration.
Plan/history storage, the prospective voice reservation, the separate 524,288-byte future Plan free-index array, complete configuration/history metadata and the 4 MiB policy workspace remain reserved in full.
No historical prepaid slack is reclaimed for the new factual free-index allocation.
The [aggregation allocation plan](adaptive-tuning-aggregation-memory.md) remains the detailed population inventory;
this section supplies the current measured deltas rather than treating the ordinary subtotal as proof of future fit.
