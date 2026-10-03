# Independent challenge

The audio/tuning investigator did not adjudicate its own primary finding.
The renderer investigator independently challenged A1 and S1; the state/scene investigator independently challenged R1–R3.
All were read-only; the coordinator owns integration and experimental validation.

## A1 — bounded storage is not bounded drain work

Challenge sustained the source mechanism and legal cross-instance overlap.
It rejected the stronger claim of demonstrated infinite production execution: Tune admission is bounded at 8192 and an injected refill controls scheduling.
A finite three-row fixture demonstrates the accepted-vs-discarded distinction without producer-rate assumptions.

Two proportional fixes remain:

- Per-row entry snapshot: finite callback work, at most 1024 per paired row, preserves existing finite backlog cleanup; arrivals during draining move to a later callback.
- Global visited-record limit: tighter total work, but valid peers can be delayed by stale backlog; preserve existing rotation and test eventual progress.

Neither requires a new scheduler, queue, handshake, persisted state or compatibility work.
Host deadline impact is unverified.

## S1 — real restore route and save freshness

Confirmed the shipping configuration-aware restore route, not the generic wrapper fallback (`plugin/lib.rs:988`, `vendor/nice-plug/src/wrapper/clap/wrapper.rs:3616`, `configuration_adapter.rs:368–412`).
Non-configuration ui-state deserialization happens synchronously off audio.
Pending restore is applied under the shared lock before input in real frame code, at open, and before close-time save.
Tests use derived serialize_fields/deserialize_fields and cover queued restores, save-before-frame and close/open order.

Qualification: 20 ms limits waiting for the editor lock only.
Serialization and another blob lock lie outside it; timeout intentionally uses the prior blob.
Tests call the first-step helper, not a native Bitwig editor.
Verdict: clean source ownership/order, freshness and total host-save latency not universally proven.
Do not add another snapshot owner without a demonstrated host problem.

## R1–R3 — keep intentional boundaries

Text paints after all prepares and rebinds earlier panes; lattice encodes each pane during prepare.
Rewriting lattice's earlier atlas-size uniform later could pair old textures with new sizes.
The difference is required by command timing, not avoidable duplicate code.
The label identity test retains old handles and reaches reuse/replacement.

Uniform split fixtures cross the real threshold and compare pixels; Solo fixtures advance hidden history and require >100 retained fading texels per layer.
Lattice resize fixtures retain GPU identity and compare byte-exact release output.
These clean conclusions survived independent source/fixture review.

Coupled spectrogram target allocation is real, but selective retention entails rebind dependencies.
Only the central halo-only experiment can add current cost evidence; shape equality alone does not prove memory identity.
No generic resource graph or blanket renderer abstraction is justified.

## Final recommendation challenge

An independent pass over the candidate decisions retained the documentation correction and a local Hub work bound as the clearest low-burden interventions.
It identified that the finite 3072-pop fixture does not distinguish the proposed per-row entry snapshot: that policy would deliberately drain the same existing backlog.
A bounded replenishment/queued-tail test belongs with implementation; the recommendation is source-reasoned, not a tested fix or a measured latency improvement.
The present audit does not introduce a production refill hook merely to manufacture a scheduling scenario.

The halo probe covers 1080p Uniform, a halo-resolution change and source-view replacement only.
It does not cover 4K, width dragging or complete frames, and its delta includes necessary halo allocation.
Extra resource dependencies would worsen maintainability; only a demonstrated user-visible performance benefit could pay that cost.

Stop condition: complete the baseline suite and narrow measurements, classify their limits, and stop unsupported redesign branches.
Native-host procedures are conditional follow-ups, not an instruction to disturb the current DAW session.

## A6 — publish only the host-visible tail length

Independent source challenge sustains the narrow representation recommendation;
native layout/lock-free probe and new fixture execution remain pending.
No measured speedup, contention, or dropout claim follows from this review.

Within the CLAP wrapper, `last_process_status` has exactly one read (`vendor/nice-plug/src/wrapper/clap/wrapper.rs:3633`),
which maps Tail(n) to n, KeepAlive to `u32::MAX`, and Normal/Error to zero.
Its only publications are construction (:709), start (:2194), and the result of an actually processed subblock (:2543).
The local result independently controls the immediate CLAP return (:2549–2557):
Normal returns CONTINUE_IF_NOT_QUIET, Tail and KeepAlive return CONTINUE, and Error returns ERROR with existing diagnostic behavior.
Do not substitute CLAP_PROCESS_TAIL or alter error handling as part of this representation edit.
VST3 has its own similarly named field and reader; the sole-consumer conclusion here is scoped to CLAP.

The locked `nice-plug-core 0.1.5` enum carries an error string slice,
and `crossbeam-utils 0.8.22` only selects primitive atomic layouts through u64 (`atomic_cell.rs:353–381`).
Its fallback store unconditionally acquires the address-hashed SeqLock (:1122–1134),
even for a Normal value whose variant itself is small.
An AtomicU32 removes that unnecessary full-status representation on supported targets.
Leaving the field alone avoids a small vendor edit and new fixture,
but retains a lock operation on every processed subblock despite no reader needing the extra data.
That trade favors the local edit without a performance benchmark because it reduces representation/synchronization burden with no new owner.
It does not justify a broader atomic-wrapper cleanup or a claim that the entire callback is lock-free.

Preserve publication placement, not just the four-case mapping.
A plugin-returned Error currently publishes zero before breaking;
an early wrapper input failure (:2297) or an invalid audio buffer (:2545) does not publish and leaves the previous tail intact.
Dedicated reset (:2224) and stop also do not clear this field; start does.
Multiple processed subblocks publish successively, with the last processed result retained.
Moving publication to callback exit or clearing on every CLAP error would create observable differences unrelated to the proposed simplification.
The wrapper currently does not call a host tail-changed notification;
changing that policy is likewise outside this representation recommendation.

Crossbeam documents Release stores and Acquire loads (`atomic_cell.rs:1075–1122`).
Use those same orderings for AtomicU32 to avoid mixing a synchronization weakening into this edit.
The inspected tail reader consumes no companion payload, so no new version, handshake, cache, or retained error string is needed.
AtomicCell<u32> is also a bounded alternative retaining the existing ordering API,
but AtomicU32 makes the actual scalar contract explicit and is already used in this wrapper.

One exported-tail fixture can cover the mapping and lifecycle using a script of statuses,
including finite nonzero Tail, KeepAlive, Normal, plugin Error, and stop/start reset;
assert both `clap_plugin_tail.get` and the immediate process return.
Include a parameter event that creates two subblocks and returns different statuses to prove last-subblock semantics,
and one early input-fault case after a nonzero tail to protect the unchanged publication boundary.
Use the existing performance fixture for Error so its expected error does not trigger the legacy debug assertion.
An allocation guard cannot detect this lock;
a mapping-helper test cannot prove exported extension wiring or publication placement.
No adversarial host scheduling harness or timing threshold is needed for this small edit.
