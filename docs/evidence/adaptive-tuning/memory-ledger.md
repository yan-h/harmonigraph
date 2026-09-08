# Aggregation storage and the future session budget

> **Frozen.**
> Measured before the [#712](https://github.com/yan-h/harmonigraph/issues/712) simplification and not maintained since.
> [The archive index](README.md) records what has changed and by how much.
> The text below is the original, with only its cross-references repointed;
> its numbers are true of the tree it was measured on and of no later one.

The tables below retain their earlier population derivation;
the later measured owner and index deltas are in [the archive index](README.md#the-terminal-fault-checkpoint-deltas).

This is the allocation account for the companion aggregation stage, its ordinary channel-wave replay slice, and original `CaptureArena` ownership —
the last two of which no longer exist.
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
| Arena Pending, including local links | 128 | 8,192 | 1,048,576 |
| Pending free index | 2 | 8,192 | 16,384 |
| Arena Birth and `Option<LifeLocal>` | 96 | 8,192 | 786,432 |
| Lifetime free index | 2 | 8,192 | 16,384 |
| Work reference, with embedded free chain | 16 | 32,768 | 524,288 |
| Ordinary `Option<OutputDelta>` journal | 112 | 4,096 | 458,752 |
| Separate emergency output journal | 112 | 128 | 14,336 |
| `Option<Manifest>` | 40 | 64 | 2,560 |
| Hub output window | 112 | 2,048 | 229,376 |
| Hub ingress window, including DIRECT | 104 | 1,024 | 106,496 |
| Source Work phase plane | 1 | 8,192 | 8,192 |
| Receiver Work permission plane | 8 | 512 | 4,096 |
| Receiver Pending permission plane | 8 | 128 | 1,024 |
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

One Source Box is 29,864 bytes, including State15,240, 64 emergency `Option<Release>` cells of 120 bytes, indices, channel waves, permits and pool headers.
Its ten backing allocations, including the arena Arc and phase plane, plus its Box total 2,905,832 bytes.
The channel owner is5,248 inline bytes, including16 wave/checkpoint/prefix owners and the existing channel history indices.
Replay adds4,504 bytes per Source Box, or76,568 bytes across all17 forwarding owners.
Pending and lifetime backing cells each grow8 bytes, adding2,228,224 measured bytes per session within their already reserved128/256-byte future ceilings.
Those physical cell increases are not a second future debit.
The complete measured ordinary increase is2,304,792 bytes per session.
Packed Work storage stays16 bytes per allocated cell;
single-target ready links reuse their existing inline envelope's otherwise unused Work-tail field.
No ready-link slab is added.
The private two-event wrapper representation keeps `Option<Group>` at232 bytes, below the separately reserved256-byte future ceiling.
The retained Stop queue occupies16 inline bytes per Source, including its reached output cut, and uses existing pending envelopes;
it adds no backing allocation or larger pool cell.
The review correction adds8 bytes per Wave and8 per Stop owner, totaling136 per Source and2,312 per session.
Wave is280 bytes and its deferred Prefix tag remains2 bytes.
Stop boundary masks stay within the existing four-u16 Header role ceiling, so Linked Pending remains128 bytes.
The joined-producer flags add eight further bytes per Source and 16 per receiver Row, or 392 bytes per session.
The retained baseline's original Coverage adds another 16 inline bytes per Source, or 272 bytes across all 17 forwarding owners;
no payload is read back after ownership transfer, and no backing allocation is added.
The new Recording disposition flags fit existing padding, so Recording and its configuration Owner stay unchanged.
Compiling the actual previous and current RecordFence definitions without test-support fields measures 32 bytes for both;
the retirement hold fits existing padding too.
ProducerJoined remains within the 56-byte Control cell and reuses the existing two-slot mailbox.
No new backing allocation is introduced.
CaptureArena adds 120 bytes per Source Box and a 64-byte arena Arc allocation per forwarding Source.
The shared intent-push counter accounts for eight of those owner bytes, or 136 bytes across all 17 Sources.
Pending, Life and Work allocated cells remain 128, 96 and 16 bytes respectively;
the lifetime assertion additionally reserves the future 128-byte resolved configuration within its 256-byte ceiling.
A Hub Box is 1,104 bytes;
each of its 16 Rows is 31,032 bytes, with 15,240 State bytes, 15,312 cached-baseline bytes and 480 remaining bytes.
Each receiver, including DIRECT, owns 5,120 permission bytes in two allocations.
Frozen traversal owns 227,056 bytes of core Scratch and 57,344 bytes of original-event metadata, with a 56-byte inline owner.
The metadata is 1,024 cells of 56 bytes, including exact original tuning bits.
The Row owner includes the finite-sweep count and ordered disposition-reply scalar.
These layout reports include test-support fields;
production assertions bound the actual non-test owners by the same charged sizes.

The configuration Owner uses 514,128 bytes in two allocations:
its 251,984-byte Box and the 262,144-byte observed DIRECT queue.
Inline owners include Timeline19,720, ConfirmedPitches16,664, LearningState2,072, Recording197,896 and Direct15,368. Recording's retained route/segment/pass storage is additional integration storage, not excluded disk history.

Two actual publication channels request 2,483,712 bytes in eight allocations.
The actual configuration mailbox requests 16,744 bytes in three allocations, including its ring and two restore slots.
Shared is 240 bytes, Update80, SourceBridge560 and HubBridge144. The process `OnceLock<Mutex<Registry>>` occupies another 26,944 static bytes, charged once globally.
The real bank of 48 rings, complete source control slots and session owner retains 6,943,936 bytes;
each ring's measured Arc/header allocation is 512 bytes.

| Actual constructor/factory measurement | Calling-thread retained bytes |
| --- | ---: |
| One Tune constructor | 2,907,005 |
| Hub constructor, including DIRECT forwarding Source and receiver Rows | 9,255,944 |
| Actual 16 endpoint triples and controls | 6,943,936 |
| Ordinary constructor subtotal | 62,711,960 |
| Full-HG exported factory | 30,743,917 |
| Full-HG activation | 760 |
| Sixteen exported Tune factories plus activation | 57,312,683 |
| Actually constructed, activated and enrolled four HG plus 64 Tune instances | 351,438,750 |
| Refused sixty-fifth Tune factory | 3,573,793 |

CaptureArena increases the ordinary subtotal by 2,588,584 bytes and the measured four-factory population by 10,354,336 bytes.
Those physical increases include growth inside already prepaid future cells.
Factory and constructor rows overlap and must not be added together.
Each fixture host adds 96 measured bytes outside the plugin.
Refused instances still construct their own owner/wrapper;
the counted registry limit is not a claim that creating arbitrary refused instances allocates nothing.
The four-session retirement interval allocated 9,216 and deallocated 303,837,104 bytes on the measured thread, returning registry counts to zero.
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
| Seventeen Source residual owners after State/emergency cells | 118,048 |
| State flags/padding for 17 source and 16 receiver copies | 264 |
| Sixteen Tune parameter allocations | 2,384 |
| Sixteen Row residual owners | 7,680 |
| Hub Box | 1,104 |
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
| Seventeen Source Work phase planes | 139,264 |
| Seventeen receiver Work/Pending permission planes | 87,040 |
| Seventeen CaptureArena Arc/header allocations | 1,088 |

The complete specified plan is 150,890,091 bytes, leaving 104,853 bytes below the unchanged 144 MiB cap.
The 230,824-byte increase over replay's 150,659,267-byte plan includes 226,304 bytes of phase/permission planes, 1,088 bytes of arena Arcs and 3,432 bytes of measured Source/Row/Hub owner growth.
That remainder is an allowance, not measured allocator-private overhead.
The attachment payload/metadata rows conservatively charge the whole process ceiling once in this single-session account;
the additional OnceLock base pins remain inside that existing metadata ceiling.
The 96-byte Intent, 72-byte Reply and 104-byte linked ingress cells remain within their existing 128/256/128-byte ceilings, including DIRECT's prepaid window.
Larger reply slots remain within the nominal 256-byte control-frame reservation too.
No larger ring cell or current ordinary-constructor increase is debited again here.

The actual 284,400-byte frozen Scratch plus metadata backing fits within the existing 425,984-byte combined cohort allocation reservation:
262,144 nominal scratch bytes, 131,072 adjacency bytes and 32,768 traversal-index bytes.
The 56-byte owner is charged in the Hub Box.
The supplemental 131,072-byte adjacency reservation is not a second new debit, and unused cohort-reservation space is not reclassified as general headroom.
All unimplemented assignment plans, histories, prospective voices, future configuration growth and the 4 MiB policy workspace remain reserved in full.

Work uses the validated sixteen-byte field split:
full 64-bit serial and 44 immutable index/operation bits in fourteen bytes, with a separately addressed two-byte Source-only ready link.
Four reachable phases occupy the separately charged two-bit plane.
There is no separate ready-link slab or Pending-to-ingress map.
The consumer-Pending to Stop-Pending association, owner masks and resolved boundary state remain Source-only;
the checked Hub view exposes the original input and captured targets instead.
Life's 96-byte allocated cell plus its future 128-byte configuration is 224 bytes, leaving 32 bytes within its unchanged 256-byte ceiling for other promised metadata.

Production assertions use the actual allocated arena/linked/Option cells, not just bare payloads.
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
