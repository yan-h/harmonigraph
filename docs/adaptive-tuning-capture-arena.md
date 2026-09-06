# Original capture ownership across callbacks

This stage follows [ordinary channel-wave replay](adaptive-tuning-channel-waves.md) and integrates the reviewed [cohort traversal](adaptive-tuning-cohort-order.md) with production-owned original input storage.
It preserves the policy engine as an unwired component.
It does not complete issue #616, install delayed assignment plans, establish musical membership/configuration frontiers, or resolve the timing work in #621.

## One original, two completion conditions

Each forwarding Source, including DIRECT, constructs one `Arc<CaptureArena>` before activation.
The arena relocates its existing Pending, lifetime and Work backing allocations;
it does not make a second snapshot of those pools.
Its registry bridge owns a base reference throughout active, offered and retired ownership.
Only off-audio registry reclamation can release the final reference.

One non-Clone Source handle owns each mutable region.
Pending separates its immutable original event, exact input serial, original inline lifetime and original Work span from its mutable channel/readiness/disposition state.
Lifetime slots separate immutable Birth identity from Source-local physical voice and reference bookkeeping.
A Work cell retains its full 64-bit lifetime serial and 44 immutable index/operation bits in fourteen bytes;
the remaining two bytes are a separately addressed Source-only ready link.
Its four reachable phases occupy a separate two-bit plane.
Code never mutably borrows a complete shared slot after publication.

The Source seals only a completely initialized input group.
Release publication transfers one token through the existing rtrb intent lane;
Acquire consumption precedes the receiver's immutable reads.
A failed publication retains the token and original cursor at the Source.
The key includes runtime lease and incarnation, epoch, input serial, arena identity and Pending position.
No output sequence or baseline acknowledgement substitutes for this key.

Local settlement unlinks channel/ready work once and marks the Pending locally done.
This allows a later physical onset or controller wave to advance while a frozen cohort still reads the earlier original.
The original Pending, complete Work span and referenced Birth cells stay pinned until exact remote retirement is also acquired.
Only then does the Source reclaim that backing and decrement its original lifetime pins.
An unpublished, unpaired input can settle locally because no receiver can read it.
After the producer has joined, local cancellation can also reclaim an unpublished original whose sample mapping prevented a token from being minted.
An offered token remains pinned even when its ring push failed;
producer join does not replace its exact retirement acknowledgement.

## Receiver and frozen ownership

Each receiver has a stable 1,024-cell ingress window and private parent/Work permission bitsets.
Installation walks the builder-certified complete original span under its unique token.
A checked `TargetAccess` lookup validates the live token, frozen input identity, lease/epoch/input serial, parent permission and Work permission before reading Work or Birth.
Its handle combines the stable ingress position with the Work or inline-parent domain;
there is no separate Pending-to-ingress map allocation.
Reusing an ingress slot, Pending cell or Birth index cannot revive an older frozen binding.
Original expression payloads preserve the native `f64` bits.

The Hub owns the cohort metadata and core traversal scratch across callbacks.
Temporary views borrow that owner, so dropping a view does not discard an offered event or permit Source reuse.
Completing traversal also leaves pins intact until the owner explicitly releases them.
The future scheduler must establish complete membership, owned input and configuration boundaries before calling these APIs.
The runtime fixtures freeze at a controlled `Hub::begin` hook, before the same callback's configuration `Owner::begin`;
that hook proves ownership only and is not the placement for future configuration freezing.

Retirement clears receiver permissions and ends its token's arena handle before Release publication of `CaptureRetired`.
A full reply lane retains the exact retirement authority in the same ingress cell.
The Source's Acquire receive validates that authority once before reuse;
duplicate or mismatched epoch, incarnation or arena keys do nothing.
DIRECT uses its prepaid ingress window and the same permission/retirement protocol within the serialized Hub owner.
Joined retired owners may abandon a frozen traversal only after callbacks have ended;
this is storage disposal, not fabricated musical completion.
A retired Hub waits for the Source's enclosing Detach boundary before ending receiver service.
A musical Seal closes an output cut but does not prevent a live Source from capturing more input before Detach.

## Bounded service and acceptance

Capture installation and retirement charge one parent visit plus its original Work count against the Hub's shared 4,096 input-work grant.
Capture, disposition and coverage publications share one Source grant of 512 successful intent pushes per enclosing callback or joined-owner service round.
Sub-blocks and repeated transfer attempts do not replenish it;
a refused push preserves its owned value and cursor.
Each Tune row and DIRECT parse at most 64 ingress cells per callback.
One scalar remaining-cell count bounds each sweep to the cells present at its start;
later appends cannot keep its cursor moving forever without revisiting an older frozen capture or blocked retirement.
Coverage and disposition can therefore pass retained capture owners without removing them.
A separate per-lease scalar preserves FIFO disposition replies when the parser wraps around a failed older reply;
it seeds from the first valid transaction in FIFO receipt order, and advances only after successful publication.
Younger dispositions stay owned until their predecessor is acknowledged.
Source cleanup retains its existing bounded visit grant and charges complete original-group cleanup.
These are work-count bounds, not a measured callback deadline or WCET claim.
The all-retired service bound reserves grant-saturated capture rounds separately from the 128-round per-row parse tail, plus retained-window revisits.
The former 1,964/2,048-round bound predates the one-record repair lane and terminal inventory.
Terminal settlement charges that serialized traffic separately;
its revised finite ownership bound does not claim callback latency or successful unknown physical termination.

The exported-factory fixtures run actual CLAP callbacks under the process allocation/deallocation guard.
They cover an offered cohort across real accepted Note-Offs and exact output ACKs in both source callback orders, original two-target wildcard traversal, Pending/Birth reuse after exact retirement, stale frozen bindings and retirement replays, a mixed-generation partially canceled wildcard, DIRECT ownership, and a full 1,024-cell retirement reply lane followed by destruction of both peers without rescue callbacks.
A reply-pressure fixture places the first of two real canceled inputs at the last visit of a 64-cell slice;
it reproduces lost disposition debt before the FIFO correction and settles both after it.
The continuous-arrival fixture starts with more than 64 retained captures and adds 64 real MIDI events per source per callback while both older Tune and DIRECT owners retire.
The mixed-generation fixture positively reaches an old disposition obligation after a newer child has settled, before the old manifest is acknowledged.
The review fixtures reproduce a 513th intent push, a retired Hub detaching before real post-Seal input, and an unpublished original stranded after sample-mapping overflow and producer join.
They verify the corrected shared grant, exact Detach boundary and joined local reclamation;
an additional backlog fixture reaches Seal while original inputs remain unpublished and drains them through the same receiver.
The [allocation account](adaptive-tuning-aggregation-memory.md) retains every future plan, history, configuration and policy reservation.
