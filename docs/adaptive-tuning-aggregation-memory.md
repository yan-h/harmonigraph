# Aggregation storage and the future session budget

This is the allocation account for the in-progress [companion aggregation branch](adaptive-tuning-companion-aggregation.md).
The hard ceiling remains 144 MiB, or 150,994,944 bytes, of additional adaptive-session storage.
It is not a limit on the whole plugin's RSS.
The current layout and constructor measurements below come from the isolated production factory fixture on macOS arm64 with the pinned toolchain.
Remaining replay and assignment integration must update the account when it adds another pool, owner or larger allocated cell.

## Current physical allocations

The measurement wraps actual GlobalAlloc allocation and deallocation requests on the calling thread.
The process allocation guard remains enabled.
It excludes allocator-private overhead, worker-thread requests/frees and RSS;
negative retirement deltas are not leak proofs.
The separate child process reaches all four actual enrolled sessions and avoids other tests collecting its global registry.

| Allocated cell | Bytes | Cells per owner | Backing bytes |
| --- | ---: | ---: | ---: |
| Pending `Linked<Pending>` | 120 | 8,192 | 983,040 |
| Pending free index | 2 | 8,192 | 16,384 |
| `Option<Life>` | 88 | 8,192 | 720,896 |
| Lifetime free index | 2 | 8,192 | 16,384 |
| Work reference, with embedded free chain | 16 | 32,768 | 524,288 |
| Ordinary `Option<OutputDelta>` journal | 112 | 4,096 | 458,752 |
| Separate emergency output journal | 112 | 128 | 14,336 |
| `Option<Manifest>` | 40 | 64 | 2,560 |
| Hub output window | 112 | 2,048 | 229,376 |
| Hub ingress window | 48 | 1,024 | 49,152 |
| Observed DIRECT output | 128 | 2,048 | 262,144 |
| Wrapper input `Option<OwnedInput>` | 176 | 2,048 | 360,448 |
| Wrapper output `Option<Group>` | 232 | 640 | 148,480 |
| Inline wrapper ready index | 2 | 640 | 1,280 |
| Publication item, each of two rings | 176 | 4,096 | 720,896 |
| Complete publication payload, each of two banks | 15,304 | 34 | 520,336 |

There are 17 full forwarding Source owners:
16 Tune sources and one DIRECT forwarding source.
Observed DIRECT input additionally owns its distinct rich State and 2,048-cell history queue.
The 16 receiver Rows each hold another rich State and a complete cached baseline.
These are actual separate allocations/copies despite the session's 256-voice musical credit limit.

One Source Box is 25,208 bytes, including State15,240, 64 emergency `Option<Release>` cells of 120 bytes, indices, channel dependencies, permits and pool headers.
Its eight backing allocations plus its Box total 2,761,848 bytes.
A Hub Box is 864 bytes;
each of its 16 Rows is 30,944 bytes, with 15,240 State bytes, 15,312 cached-baseline bytes and 392 remaining bytes.

The configuration Owner uses 514,128 bytes in two allocations:
its 251,984-byte Box and the 262,144-byte observed DIRECT queue.
Inline owners include Timeline19,720, ConfirmedPitches16,664, LearningState2,072, Recording197,896 and Direct15,368. Recording's retained route/segment/pass storage is additional integration storage, not excluded disk history.

Two actual publication channels request 2,483,712 bytes in eight allocations.
The actual configuration mailbox requests 16,744 bytes in three allocations, including its ring and two restore slots.
Shared is 240 bytes, Update80, SourceBridge544 and HubBridge128. The process `OnceLock<Mutex<Registry>>` occupies another 26,944 static bytes, charged once globally.
The real bank of 48 rings, complete source control slots and session owner retains 5,894,848 bytes;
each ring's measured Arc/header allocation is 512 bytes.

| Actual constructor/factory measurement | Calling-thread retained bytes |
| --- | ---: |
| One Tune constructor | 2,763,005 |
| Hub constructor, including DIRECT forwarding Source and receiver Rows | 7,714,856 |
| Actual 16 endpoint triples and controls | 5,894,848 |
| Ordinary constructor subtotal | 57,817,784 |
| Full-HG exported factory | 28,153,741 |
| Full-HG activation | 760 |
| Sixteen exported Tune factories plus activation | 55,008,683 |
| Actually constructed, activated and enrolled four HG plus 64 Tune instances | 331,862,046 |
| Refused sixty-fifth Tune factory | 3,429,793 |

Factory and constructor rows overlap and must not be added together.
Each fixture host adds 96 measured bytes outside the plugin.
Refused instances still construct their own owner/wrapper;
the counted registry limit is not a claim that creating arbitrary refused instances allocates nothing.
The four-session retirement interval allocated 9,024 and deallocated 284,766,728 bytes on the measured thread, returning registry counts to zero.
Other-thread ownership remains outside that counter.

The sixteen new companion wrappers have 2,635,739 bytes of measured additional owners and generic queues after subtracting their constructors, performance boundary arrays/ready indices and fixture hosts.
These new copies are chargeable even though the full plugin already had a wrapper before adaptive tuning.
They include the wrapper's actual fixed task and output-parameter queues.

## Constrained future allocation plan

The engineering contract's nominal payload subtotal remains 132,716,032 bytes.
It reserves the absent assignment plans, histories, prospective voices, DIRECT ingress, cohort scratch and 4 MiB policy workspace in full.
The current ordinary subtotal does not prove those future implementations fit.

The following costs are additional to those nominal payload rows.
Future growth within an already reserved allocated-cell ceiling is charged once;
another physical copy or separate index pool is not free.

| Additional physical pool | Bytes charged |
| --- | ---: |
| All 17 work-reference slabs | 8,912,896 |
| Seventeenth forwarding Source pending/lifetime/journal at nominal ceilings | 3,670,016 |
| All pending/lifetime free-index vectors | 557,056 |
| Separate emergency output journals at 128 bytes/cell | 278,528 |
| Sixteen new Tune wrapper residuals | 2,635,739 |
| Sixteen receiver voice sets at 256 bytes/cell | 262,144 |
| Sixteen receiver channel arrays | 38,912 |
| Sixteen cached complete receiver baselines | 262,144 |
| Distinct rich observed-DIRECT voice/channel state | 18,816 |
| Additional physical-pool subtotal | 16,636,251 |

This leaves 1,642,661 bytes before accounting for the remaining owner and future bookkeeping rows below.
The 8.5 MiB reference debit has already consumed part of the nominal 16 MiB overhead allowance;
16 MiB is not still available for unrelated future pools.

| Remaining owner/metadata reservation | Bytes charged |
| --- | ---: |
| Seventeen Source residual owners after State/emergency cells | 38,896 |
| State flags/padding for 17 source and 16 receiver copies | 264 |
| Sixteen Tune parameter allocations | 2,384 |
| Sixteen Row residual owners | 6,272 |
| Hub Box | 864 |
| Source control/Arc headers beyond nominal frames | 1,152 |
| Source baseline slot atomic headers | 256 |
| All 48 ring headers | 24,576 |
| SessionControl Arc and HubBank | 1,344 |
| Seventeen Shared/update-slot Arcs | 7,616 |
| Future Update payload growth from 80 to 256 bytes | 5,984 |
| Entire process attachment payload ceiling | 69,632 |
| Entire process attachment slot/Arc/extra owner metadata ceiling | 4,416 |
| Static Registry wrapper | 26,944 |
| Recording route/segment/pass owner | 197,896 |
| LearningState and other configuration Owner fields | 2,336 |
| Timeline and confirmed-owner headers | 544 |
| Observed DIRECT owner outside State, including State padding | 136 |
| Configuration mailbox metadata, including conservative slot growth | 1,144 |
| Publication Shared/ring metadata | 1,248 |
| Future extra resolved-configuration growth in recording/timeline headers | 16,512 |
| Full-HG incremental wrapper/recorder/adapter reservation | 65,536 |
| Future plan free-index array, `131072*u32` | 524,288 |
| Future cohort adjacency matrix, `1024*1024` bits | 131,072 |
| Future cohort ready/degree/traversal indices | 32,768 |
| Future remaining fixed plan/history/source metadata | 65,536 |

The complete specified plan is 150,581,899 bytes, leaving 413,045 bytes below the unchanged 144 MiB cap.
That remainder is an allowance, not measured allocator-private overhead.
The attachment payload/metadata rows conservatively charge the whole process ceiling once in this single-session account;
HubBridge's 16-byte shrink therefore does not reduce those reserved ceilings.
The separate policy/cohort drafts still need an integration check against these exact reservations, including transport ownership that is not present yet.

Production assertions use the actual allocated Indexed/Option cells, not just bare payloads.
Pending's 128-byte ceiling includes its links;
its free-index vector is charged separately above.
Whole protocol, cached and publication baselines must fit 16 KiB while their nested voice/configuration fields also fit their own maxima.
A 64-voice frame cannot grow every nested voice to 256 bytes and still have room for channels in 16 KiB.
Both constraints apply simultaneously.

The complete full-HG Wrapper is currently 9,216 bytes, including its inline runtimes and scheduler indices.
The additional setup adapter and weak-owner wake closures are small fixed allocations;
the recorder lifetime guard clones its existing command sender and creates no second ring.
The 65,536-byte incremental-owner reservation conservatively includes the whole wrapper and those small additions, beyond the separately charged arrays, Shared objects and configuration mailbox.
It must be revisited if those interfaces acquire another pool.

## Whole-plugin exclusions

After subtracting the counted Hub, endpoint bank, performance arrays/ready indices, configuration Owner, publication channels, mailbox and fixture host, the full-HG factory/activation residual is 11,019,909 bytes.
Known existing payloads account for 10,485,760:
the 65,536-entry take ring at 88 bytes/entry, the 4 MiB audio recording ring and the 524,288-byte analyzer audio ring.
The remaining 534,149 includes their headers and existing analyzer/UI/parameter/framework owners together with small integration fields.
The separate 65,536-byte incremental reservation prevents silently excluding the latter.
The entire residual is not classified as existing storage.
Worker file histories, fanout maps and native Cocoa/GPU allocations remain separate integration/RSS observations.

Final measurements are still owed after delayed replay and assignment integration stabilize their layouts.
