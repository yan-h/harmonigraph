# Adaptive tuning ordinary progress

This stage repairs normal input/output progress over terminal checkpoint `5599f19dfa7607ba9e2ab70a9272b03d24118cfa`, the reviewed but unmerged draft [#688](https://github.com/yan-h/harmonigraph/pull/688).
It addresses [#685](https://github.com/yan-h/harmonigraph/issues/685), [#686](https://github.com/yan-h/harmonigraph/issues/686), [#689](https://github.com/yan-h/harmonigraph/issues/689) and the relevant fixture expectations in [#690](https://github.com/yan-h/harmonigraph/issues/690).
The [engineering contracts](adaptive-tuning-contracts.md) remain authoritative.
Normal Tune delay remains 512 samples;
ordinary lateness and bounded transfer/work pressure still retain notes.
Automatic ordinary divergence activation and production musical-policy integration remain later stages.

## DIRECT input and forwarding

An initially uncalibrated standalone full Harmonigraph now validates its own negotiated rate, callback size, nonnegative checked steady time and continuous local interval before forwarding at zero delay.
Its local clock never marks external routing calibration valid, opens a tuner membership or supplies an adaptive assignment.
Observed DIRECT input and actual accepted forwarding remain separate facts.
An invalid local callback latches the selected clock fault;
an already calibrated route cannot fall back to this initial local mode.
Explicit Reset waits for the existing sealed ownership/recording/credit boundary and a fresh callback, then can restore the same never-calibrated standalone mode.

Initial factual Capture tokens retire through the existing bounded permission/retirement path without waiting for a sequencer that cannot use an uncalibrated route.
Calibrated DIRECT instead exposes the exact transferred complete input prefix:
exclusive coverage stops at the first untransferred Original's mapped sample, and its cut is checked against the Hub's captured sequence.
It does not certify that sample merely because some earlier events at that sample transferred.
Older complete cohorts can finish and release their ingress ownership while younger Originals remain Source-owned.

## Factual lookup and clock boundaries

Factual slots no longer borrow their only reusable hint from musical Plans.
A `17 * 16 * 128` directory of `u16` hints addresses current source/channel/key voices, and a separate 256-entry key array validates the exact session, SourceId, incarnation, slot, epoch and lifetime for every hit.
The existing one-current-voice-per-channel/key State contract supplies the address uniqueness;
the directory does not replace lifetime identity.
DIRECT uses its observed-input lifetime, which is independent of forwarding's Source Life indices.

A lookup spends one work unit for its hint check.
An unavailable or mismatched hint reserves the full 256-unit fallback scan before running it, unless this is a new onset.
The copied found slot or explicit absence passes directly to application, including unmapped terminals and DIRECT;
application never repeats an uncharged search.
No grant was enlarged and aggregation tests still ingest factual output without manufacturing Plans.
Changing a voice's expression or `actual_revision` does not invalidate its stable identity hint.
A lifetime-modulo 64 cache was rejected because legitimate survivors 1 and 65 could force a scan on every alternating expression.

Release and baseline replacement invalidate the slot keys and refresh affected directory entries.
A complete two-voice baseline can reverse both physical slot indices;
source State's packed array is not an index authority either.
Stale hints are harmless after reuse because the full key must still match.

At a committed clock change, all old cuts, readers, rows and physical credits settle before the factual/prospective cache is replaced.
The transition reserves 256+64 output-work units and a checked revision before committing the recording clock.
After `Owner::resume_clock` chooses its observed-input state, the cache is cleared and that authoritative DIRECT state is reseeded under the new epoch.
A discontinuous Reset supplies an empty observed set;
a healthy reanchor preserves its lifetimes and exact baseline player expression.
This emits no On and claims no forwarding credit.
The synchronous idle host-Reset path also clears observation-cache identities, including when no Hub offer exists, without changing independent forwarding debt or consulting the preceding callback's work counter.
Neither path scans the 34816-entry hint directory.

## Reaching evidence

The guarded focused plugin tests explicitly enable `nice-plug/assert_process_allocs`, use this worktree's target, two Cargo jobs and two test threads.
The configured sccache failed with sandbox `Operation not permitted`;
the authorized `RUSTC_WRAPPER=''` fallback was used.
Only one local Cargo build/test/Clippy process ran at a time.
One additional exploratory `reset` filter omitted the allocation feature;
it is preserved separately and is not a guarded receipt.
Its two primary channel-fixture failures were then reproduced individually with the feature enabled, while the later production-reset failure passed in isolation after avoiding the leaked state cascade.

The unchanged sixteen-source fixture failed alone at the base with 256 credits instead of zero.
Instrumentation then showed all four contributing rows had received sequence 128 with coverage through 192, but their applied cuts were 77/64/64/64:
243 accepted Offs were still queued.
The Hub had spent 3853 units (`512 + 13 * 257`), so the complete frontier remained 128, excluding the actual release sample 128. All four Source retention ACKs were 128 and their journals were empty;
retaining the credits was correct.
After the index fix, the same workload spent 768 units, applied all four cuts 128, advanced the frontier to 192 and returned every credit.

The original publication-loss/repair tests and original 48-by-200 nonhead workload now pass unchanged.
The nonhead fixture previously accepted 28 events at its failing batch instead of 200;
the fixed run retains its older blocked attack while processing the complete stress stream and exact release.
The two publication fixtures still reach real 4096-cell loss and verify exact lifetime repair and independent display/disk outcomes.

The new production DIRECT fixtures independently assert actual On/expression/Off acceptance at samples 3/11/19 and ObservedDirect publication, including the exact f64 expression.
They account separately for activation's expected SourceReset at time zero, sequence/lifetime zero and no mapped timing.
A 1536-Original calibrated workload across three 512-frame callbacks drains through continuous empty callbacks without Reset, fault or retained ownership.
The measured distinct-sample run needs 23 empty callbacks.
The first draft used an unsupported 2048-frame fixture callback and failed before the target branch;
the corrected fixture then failed at missing complete-prefix readiness before the implementation fix.

Reset coverage found an additional stale DIRECT cache entry after the observed state had been cleared:
the old lifetime at the unsupported gap survived a committed fresh epoch.
The corrected discontinuous Reset test verifies its removal, while the healthy reanchor test verifies preserved observed pitch, subsequent tuning and exact input-Off removal without a new On.
The factual baseline fixture proves both physical slots swap before checking independent expression, release and same-key reuse.
The sparse-lifetime/missing-identity test verifies charged fallback and explicit absence across session, SourceId, incarnation, epoch and lifetime mismatches.

The unchanged partial-setup Stop fixture now reaches its 512 accepted prelude events from the factual-throughput fix alone.
The other Stop fixtures retain their stress counts and exact canceled-prefix assertions, but treat a real rejected Choke/repair as terminal inhibition until Reset.
Fresh post-Reset input waits for the requested host main callback to return/rematch the settled Source lease and establish the fresh baseline.
The 641-event Stop fixture fills the actual single-disposition repair lane with 644 Capture owners, 131 local pending obligations, 512 journal records and zero retention ACK.
Its complete cut drains in 130 callbacks, preserving exactly the 512 accepted prefix events and one new stopped-live controller event.
The Stop group passes 9/9. The production group passes 34/34 and Capture group 10/10;
these are focused functional/ownership receipts, not complete timing or full-feature qualification.

## Memory and remaining scope

The isolated allocation fixture measures 106209688 ordinary bytes and 424838752 for four sessions.
Source owner size is 31288, Sequencer 5840, Row 31128 and SourceControl 31248. Plan Option remains 192 bytes with 25165824 bytes of physical backing;
the factual Voice Option remains 40 bytes.

The complete added ownership is 84544 bytes:

| Added owner | Bytes |
| --- | ---: |
| `34816 * u16` factual address hints | 69632 |
| `256 * 56` exact optional keys | 14336 |
| `17 * 32` local-clock Source header growth | 544 |
| Sequencer Box metadata growth | 32 |

Adding 84544 to the audited future 150646323 gives **150730867 bytes**, with **264077 bytes spare below 144 MiB**.
All prepaid 256-byte Plan/Life cells, configuration expansion, confirmed/prospective voice and history reservations, the separate future Plan free-index array and 4 MiB policy scratch remain intact.
No earlier prepaid slack was reclaimed or counted twice.

[Issue 692](https://github.com/yan-h/harmonigraph/issues/692) preserves a distinct invalid-routing Reset probe:
an invalid proposed transition retains the head and blocks a later valid Reset.
Its exact request sequence and eliminated hypotheses are recorded there;
invalid-route supersession is not implemented here.
The normal valid Reset and initial local Reset paths are independently covered.

[Issue #693](https://github.com/yan-h/harmonigraph/issues/693) preserves two additional channel Reset fixture failures.
The prefix-association fixture fails at the same input-cut assertion on the exact base and candidate.
The rejected-setup fixture fails earlier on the base before its expected output;
the index fix reaches that output and then a later fault-clearance assertion fails.
This distinction is a newly reachable path requiring follow-up, not evidence that the same later assertion already failed on the base.
Neither fixture was changed here, and full Reset/CI clearance is not claimed.

This stage does not qualify the complete feature:
automatic ordinary divergence activation, retained-performance reconciliation, production musical-policy/history/metadata integration, maximum full-policy cost and practical Bitwig/D512 validation remain required by #616/#621. The inherited owner-loss and atomic delivery-generation corrections remain in place.
The release handoff and exact committed-head Actions/review receipts accompany the draft PR;
the shared Bitwig slot is not changed by this work.
