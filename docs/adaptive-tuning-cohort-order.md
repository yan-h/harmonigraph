# Canonical same-sample cohort traversal

This stage implements the reusable, unwired ordering component of [#616](https://github.com/yan-h/harmonigraph/issues/616) in `harmonigraph-core::cohort`.
It builds a dependency graph and exposes one canonical event phase at a time.
It does not implement the central sequencer, artificial production policy, tuning output, input collection, configuration binding, revocation or recovery.
The reviewed pure musical policy is its base, not something this module invokes.
Neither #616 nor #621 is complete.

The selected behavior follows [engineering contract section 5](adaptive-tuning-contracts.md#5-configuration-and-canonical-input-order).
The accepted production delay candidate remains D512 at 44.1 kHz;
this component supplies no callback throughput, latency, host-output or Bitwig evidence.

## Ownership and API

The future central owner first validates its fixed membership snapshot, leases, source incarnations, clock epoch, mapped sample and complete source coverage.
It applies configuration markers at that sample and freezes one configuration for the entire cohort.
Those are caller responsibilities:
matching `Event.sample` values cannot prove a complete frontier.
Source ordinal zero is DIRECT and ordinals 1 through 16 are the enrolled tuner tie order.
An `InputId` is the original source ordinal and nonzero input sequence inside that already validated binding.
The module neither allocates identities nor uses host note IDs to invent lifetimes.

Each original `Event` contains its identity, sample, semantic kind and compact captured target spans.
The original performance payload stays with its existing owner and is retrieved by the returned input identity/index.
`Kind::Tuning` preserves the original finite CLAP f64 value bits;
the traversal does no f32 conversion, pitch arithmetic or expression composition.
The adapter classifies unrelated system MIDI as `Independent`, shared-channel MIDI/pedals as `Channel`, and targeted notes/expression using their captured lifetimes.
Channel termination is a semantic effect on its captured targets, never evidence of accepted host output or neutralized pedals.

`TargetAccess::get(source, handle)` reads one immutable captured reference in O(1), without allocation or side effects.
A `TargetSpan` has a stable u32 handle, an explicit length at most 64, and links ending at `NO_TARGET` exactly at that length.
Handles are local references, not runtime identities.
Targets contain a source-local lifetime, exact original host note ID, channel and key.
Their chain can be noncontiguous and in any order;
the module sorts only one event's targets into its own 64-element scratch array.
Empty captured wildcard sets stay empty, and later sounding state is never consulted.

The borrow must cover a frozen input owner, separate from mutable prospective context/history.
It pins both the event metadata and target view across construction, partial traversal, offered events and reset.
It must not borrow a tuner's concurrently mutable `Source` or its current held set.
The current aggregation source owns linked `source/work.rs` cells and mutable lifetime records, while its `Intent` protocol currently carries coverage/disposition messages rather than these sequencing inputs.
Those cells are not already an immutable Hub target view.

The later Source-to-Hub transport must publish original captured metadata into a safely owned/pinned representation and retain it until the Hub releases every graph/input reference.
The Source's output obligations may outlive that graph, so its own retention and the Hub's retention require distinct completion conditions.
An adapter can expose linked immutable reference storage through this API without sorting or copying a target array per event, but it must implement and account for that storage and transfer protocol.
This stage provides no such transport and does not prove its whole-session memory fit.
In particular, copying 1,024 arrays of 64 `Target` values would require 1,048,576 bytes before wrappers and would exceed the remaining session headroom.
The test's large frozen capture owner is fixture setup, not an approved production allocation.

Construct `Scratch` once outside processing, then call `Cohort::begin(sample, events, target_view, scratch)`.
`advance(work_budget)` performs at most that many structural work units and returns one of:

- `Pending`: retain all borrows, configuration and traversal state for the next slice.
- `Event(Selected)`: inspect `event_index`, original `id`, `phase`, `role` and optional exact `initial_tuning`.
- `Complete { original_inputs, vertices }`: every offered phase has been committed and its successor edges retired.

An offered event remains identical across repeated `advance` calls, including a zero-budget call.
The caller applies the event to prospective context, evaluates any required tuner assignment, and incorporates its assignment/history before calling `commit()`.
Only then may traversal select another event.
`ObservedDirectOnset` is explicit observed context and requires no tuner plan;
there is no automatic musical resolver call for any role.
Commit means the caller incorporated a canonical phase, not that a host accepted output.

`was_committed(original_index)` and `was_replacement_committed(original_index)` distinguish an original event from its earlier replacement-release phase.
The `committed` and `remaining` counts refer to graph phases;
the total phase count becomes final during construction, before the first event is offered.
`reset()` restarts the same immutable cohort and incrementally clears/rebuilds graph state on subsequent `advance` calls.
It cannot roll back external assignments or accepted output;
the owner must separately discard or revoke prospective effects before replaying the graph.
Dropping the traversal releases borrows and performs no allocation or deallocation.

## Dependencies and replacement phases

Edges follow original source sequence for shared lifetimes, old termination before same-key replacement, and shared-channel controls.
There is no edge between unrelated onsets merely because one source delivered them in that order.
Every graph edge points forward in `(source ordinal, input sequence, phase)` order, so a valid graph is acyclic by construction.
There is no public arbitrary-edge API;
`Cycle` is a defensive failure when traversal has remaining vertices and no ready one.

Ready non-onsets take precedence, ordered by source ordinal/input sequence, with a replacement release before the original phase of that input.
Ready onsets use key, channel, source ordinal and input sequence.
A smaller-key onset cannot pass its prerequisites.
An off after a same-sample on remains after its on, including a zero-duration lifetime.
Healthy host output still follows its original per-source order at input plus D;
the canonical evaluation order is not an output schedule.

A replacement onset may carry one captured old same-key target in `Event.replaced`.
The graph gives that physical input a virtual `ReplacedRelease` phase followed by its `Original` onset phase, preserving the same input identity and index.
The old target must have the same channel/key and a different lifetime;
its original host ID can differ from the new lifetime's host ID.
Both phases have their own dependencies, and the release always precedes its own onset.

For example, an old lifetime's pitch update at sequence 9 precedes its replacement release at sequence 10. A captured old-lifetime off at sequence 12 can then run before the replacement attack, while an independent key-40 attack at sequence 11 wins key order ahead of the replacement's key-48 attack.
The fixture's visitor removes the old context at the release phase and incorporates the key-40 assignment before evaluating key 48. Collapsing both phases into one graph vertex would incorrectly hold old-lifetime successors behind the unrelated new attack.

For each onset, construction records the first tuning event in original sequence that targets its own lifetime, follows the attack, and precedes that lifetime's first terminal boundary.
It returns the tuning event's exact input identity, original index and value bits.
All original tuning events remain in their normal dependency/source order, including subsequent updates at that sample.
No eligible event yields `None`;
the caller retains its own onset-expression default.
An event from a later sample makes the input invalid instead of donating an expression value.

## Physical bounds and reserve reconciliation

`COHORT_EVENTS = 1024` counts original physical inputs, and `COHORT_ONSETS = 256` counts their onsets across all 17 ordinals.
Up to 256 of those onsets can have one virtual pre-release phase each, so `COHORT_PHASES = 1280` bounds graph vertices without admitting more original inputs.
Wildcard and channel messages still own one original slot with their captured target set.
This does not reinterpret the Source's 8,192 pending-event capacity as expanded work.

The approved internal split keeps the previous total cohort reserve of 294,912 bytes unchanged:

| Reservation | Previous split | Component split |
| --- | ---: | ---: |
| Original metadata cells | 1024 × 128 = 131,072 | 1024 × 56 = 57,344 |
| Adjacency bits | 1024 × 1024 / 8 = 131,072 | 1280 × 1280 / 8 = 204,800 |
| Degrees, phase indices, target scratch and traversal wrapper | 32,768 | 32,768 |
| Total | 294,912 | 294,912 |

The 73,728-byte metadata reduction pays exactly for the larger phase graph.
Compile-time assertions cover the actual `Option<Event>` cell, full `Scratch`, traversal wrapper, and aligned composite layout.
Executed aarch64 sizes are `Event = Option<Event> = 56`, `Target = 16`, `Scratch = 226816`, `Cohort = 256`, and scratch alignment 16 bytes.
The complete aligned metadata/scratch/traversal composite is 284,416 bytes, leaving 10,496 bytes within the existing cohort reserve.
The session's planned 150,581,899-byte total and 413,045-byte remaining headroom to 144 MiB therefore do not increase.
Later integration must reconcile this internal split in its full owner ledger and account for all additional captured-target transport owners.

The 1,025th original input fails `begin` with `TooManyEvents`.
The 257th onset fails incremental preparation with `TooManyOnsets`, before graph construction can offer any event.
Target count, malformed/truncated chains, invalid source/sequence/sample/channel/key/host-ID metadata, duplicate original input IDs, duplicate target lifetimes, conflicting lifetime addresses, duplicate onsets and capture of a lifetime before its own onset produce explicit errors.
Errors latch for that traversal and never report partial completion.
The module cannot detect omitted physical inputs, omitted captured effects or invalid lease/clock provenance without the caller's upstream evidence.

## Executed work and verification

The algorithm charges each original-input preparation, cleared adjacency word, node/target validation, node pair, target read, target comparison/shift, finalized node, ready scan, retired adjacency word and retired edge.
`Work::units()` sums those operations;
edge insertion and commit totals are also exposed.
One unit includes a bounded amount of scalar work, at most one target-provider lookup, and fixed bookkeeping such as clearing a 16-entry channel/key mask.
Construction retains its row, linked-target, insertion and binary-search cursors.
Traversal retains its ready-scan position and edge word/bit cursor.
Commit, stable-offer inspection and reset are constant work and use no target lookup.

Before any event is offered, all input/target validation and all graph construction complete.
The one-unit resume fixture produces the same order and exact work counters as an unrestricted traversal.
Every fixture slice also checks that its `Work::units()` delta is no greater than the supplied budget.
The caller still needs a measured conversion from structural work units to its enclosing callback budget;
`HUB_EVENT_WORK = 1024` is not permission to build this entire graph in a callback.

The maximum replacement fixture has 1,024 physical inputs, 256 replacement onsets and 1,280 visited phases, using four source-held sets of 64. Its 768 shared-channel controls create 393,088 real dependency edges.
It executes 818,560 node pairs, 1,638,400 ready-scan visits, 25,600 retired words and exactly 393,088 retired edges.
It builds, traverses, resets and traverses again under an allocation/reallocation/deallocation guard, and exercises 1,025-input and 257-onset refusal with no offered or committed event.

The maximum-target fixture has 1,024 messages targeting 64 held lifetimes each through reverse-ordered, noncontiguous chains.
Every event depends on every preceding event, producing all 523,776 possible forward edges.
It tests both a shared immutable chain and distinct physical reference chains containing equal lifetime/address sets.
The equal-span fast path is restricted to two non-onsets with exactly identical source/span references:
the same immutable cells cannot disagree about their addresses, and per-row validation still runs.
It reduces that shared-chain case to 65,536 target reads and 2,129,920 target comparison/shift steps.

The distinct-chain case still executes 33,587,200 target reads and 173,928,448 target comparison/shift steps, plus preparation, validation and traversal work.
Skipping remaining target checks after finding one dependency, or merely because a shared channel already establishes an edge, would conceal conflicting lifetime addresses, so that shortcut was rejected.
These executed counters prove finite, resumable work and zero heap calls, not practical D512 throughput at all stored limits.
Production integration must assess the actual admitted capture shapes and complete callback workload before selecting a work slice or making a latency claim.
No timing benchmark or WCET claim was made.

Nine focused semantic tests execute the real graph/traversal for all six three-source arrival permutations with a context-dependent assignment visitor, all 17 ordinal ties including DIRECT, established pitch/release updates, zero-duration notes, replacement phases, shared-channel pedals, immutable wildcard sets, exact initial expression, partial resume/reset and invalid metadata/chains.
The separate `cohort_heap` integration-test binary owns its allocator guard so the existing policy tests' allocator and source files remain untouched.
Its fixture setup and capture allocation occur outside the measured region.
Rust formatting, core clippy and focused tests are the local gates;
the draft PR runs the repository's full GitHub Actions gate on the committed head.

## Remaining central integration

Restack onto the independently reviewed aggregation and combined sequencing base before integrating this module.
The central owner still owes complete chronological collection/frontiers, original target transport/retention, configuration freeze and request binding, prospective context/history ownership, the artificial production-policy proof, issued plans, suffix revocation and accepted-output recovery.
Source scheduling still owes D512 delayed full-stream output, exact expression composition, late waves, Stop/Off/emergency behavior and the accepted-output ownership boundary.
The full-session byte ledger, real callback allocation/free guards, nonempty history workload, complete central/source callback cost and calibrated boundary schedule remain required.
The pure musical policy's final wiring and D/F/A production sequencing fixture remain separate integration work.
No live/offline Bitwig timing, compensation, routing or destination-pitch validation has been performed by this component stage.
