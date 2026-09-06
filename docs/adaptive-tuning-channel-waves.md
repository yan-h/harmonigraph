# Ordinary channel waves and input-history replay

This slice extends the production companion forwarding owner from [aggregation](adaptive-tuning-companion-aggregation.md).
It covers reconstructable input state and delayed ordinary waves;
it does not complete the assignment/cohort integration in #616/#621 or settle the separate product decision for overwritten unknown initial receiver state.
It neither invents that initial state nor adds a Reset-required recovery policy for it.

## Translation and original ownership

Each accepted onset fixes its lifetime's translation from original input to actual output.
One channel wave owns the compatible translation currently using the receiver.
A differently translated younger onset waits until older channel voices, physical Off bindings and release debt drain, and CC64/66/69 are known accepted neutral.
Free session credits alone do not establish that boundary.
The wave changes on accepted onset, not on a staged or rejected attempt.

Established expressions and releases use their own accepted lifetime shift.
Original channel controls can reach the established wave while younger captured targets remain unsounded.
Their raw envelopes remain available for the younger wave, including zero-target setup that preceded its onset.
Replayed controls retain their original relative offsets after that onset;
sostenuto therefore receives its edge after the originally preceding note rather than a reconstructed final pedal snapshot.

Pending owns the unchanged original event, sample and serial.
Work owns captured lifetime references for channel controls and multi-target addressed envelopes.
Single-target addressed input retains the existing inline fast path.
Per-lifetime readiness links address Work 0..32767 or tagged Pending 32768..40959, with 65535 as the sentinel.
Full 64-bit identities remain intact.
A ready link pins its parent and lifetime until unlinking, including cancellation acknowledgements and accepted releases that bypass earlier unfinished work.
Indexed reserved and separately owed physical Off owners remain responsive across a retained prefix within the 8,192-envelope pool.

The callback retains its 2,048-visit budget.
Original capture preflights the whole reference group.
The inline path preserves the existing 400-expression and 512-normal-attempt fixtures;
indexed onset wakeups cannot consume staging allowance ahead of an earlier addressed envelope.
Empty cancellation cuts settle immediately, while nonempty cancellation still waits for traversal, manifest acknowledgements and ready-link cleanup.

## Setup, transactions and accepted facts

A per-channel folded input checkpoint is separate from State's actual accepted receiver state.
The channel history cursor retains original controls after the oldest unsounded onset.
Beginning a new wave freezes that checkpoint and replays its required prefix incrementally, then rewinds the retained wire cursor to the younger onset's input cut.
Acceptance of setup cannot fabricate an onset or reserve a sounding lifetime.

Program/bank, RPN/NRPN and relative data-entry operations retain a chronological raw prelude in the existing bounded Pending pool.
The implementation does not sort those messages into independent CC registers or guess device-specific NRPN arithmetic.
Bank selection preceding a program change and later pending bank selection remain in their original order.
The retained prelude consumes the same finite 8,192-envelope capacity as other original input;
it is not an unbounded secondary history allocation.

Every actual setup event uses the normal prepare/host/complete path with journal headroom reserved before the host call.
Each separate setup validates its anchored pending serial and generation, checks current admission, and claims the real lease gate from OPEN to BUSY.
The claim lasts through accepted State and journal bookkeeping;
a concurrent close can let that claimed group finish but inhibits the next group until valid readmission.
Setup never acquires a musical voice credit.
An accepted prefix remains factual if a later setup event is rejected.
Emergency pedal neutralization updates actual state and its corresponding input checkpoint repair separately;
a later Reset cannot resurrect an already overwritten input pedal value.
Both raw replay and checkpoint folding substitute repaired pedal values only for Control Change messages;
pitch-bend bytes that happen to match a pedal controller number remain unchanged.
Preparation checks the captured Stop sample even when the visit budget cannot yet advance the Stop marker.

CC120/123 remain physical channel operations with input-captured targets.
Each wave's physical acceptance settles only targets actually sounded in that wave.
CC120 additionally retains the original physical Note-Off binding after the logical terminal and its credit acknowledgement.
The owed-only release index therefore remains live after the lifetime leaves the reservation array.
Reports distinguish one physical wire acceptance from its explicitly linked per-lifetime terminal facts.

Delivered reconstruction history is not musical pending work.
Acknowledged neutral musical idle can clear extra delay while a raw prelude remains retained.
Producer retirement releases retained reconstruction history through the existing bounded cancellation/cleanup path.

## CC88 belongs to its next raw MIDI consumer

The [MIDI Association's CA-031](https://midi.org/high-resolution-velocity-prefix) defines the next matching-channel raw MIDI NoteOn/Off as the consumer, permits intervening non-note messages, and clears the receiver's low seven velocity bits after consumption.
That specifies known post-consumption zero;
it does not establish zero for an unknown live receiver at startup.
Native CLAP note/choke events and Stop are not treated as raw MIDI consumers.

An ordinary CC88 still emits at its healthy original time, even when the following note has not yet been captured.
Each original MIDI note snapshots its input prefix association separately from the actual receiver latch.
Setup replay skips consumed CC88 history.
If a responsive older Off would consume a delayed younger note's prefix, a validated CC88/raw-note pair reconciles the prefix immediately before that Off;
the delayed note restores its own prefix when eligible.

The wrapper accepts only two specific pair families:
NoteOn/tuning and CC88/raw MIDI NoteOn/Off on matching port/channel.
Both reuse one private two-event representation and the existing consecutive host-call loop.
Preparation reserves the dependent note's credit and all report cells before either call.
Completion records accepted CC88 with no lifetime, then records a note only when its separate acceptance bit is set.
If the prefix alone was accepted, the unused onset reservation is returned exactly once, while actual prefix history, visible output fault and channel repair debt remain owned.
Every new partial acceptance rearms its own repair even when OUTPUT_FAULT was already latched.
No fictitious On or terminal is created to refund a preflight reservation.

An accepted raw MIDI consumer derives receiver-prefix zero without adding a wire reset record.
A rejected consumer leaves the accepted prefix unchanged.
Emergency prefix repair precedes any raw MIDI Off that could consume it;
rejection retains the debt and the bounded retry must accept before that Off is attempted.
The reserved 128-attempt allowance fits 64 voice terminations, 48 pedal resets and 16 prefix repairs.
Rejected attempts can leave debt for a later callback, and accepted repair records remain in the emergency journal until retention acknowledgement.
Without a valid clock mapping, the same accepted nonzero corrective prefix, emergency zero and raw Off still update both Source and Hub receiver state and settle their factual sequence acknowledgements.
They do not acquire invented musical timestamps.

Input capture can run ahead of Stop output, so the first subsequent raw consumer holds a deferred association to that exact Stop envelope.
A two-byte tag replaces the existing two-byte optional prefix;
a channel owner bit pins the original full-serial Pending cell against reuse, then transfers from the Wave register to its first consumer.
Explicit later CC88 input overrides the Wave register without rewriting a captured consumer's association.
A reached Stop leaves the scheduling list while its known/unknown boundary result remains pinned.
Known nonzero boundary state waits for actual accepted neutralization;
known zero stays zero and unknown stays unknown.
Accepted repair authority retains the reached Stop cut, so a later prefix or later Stop cannot change an earlier boundary result.
Resolved Wave registers release their pins immediately;
consumer cleanup, cancellation and producer retirement release each remaining pin exactly once.

The same Stop cancellation disposes old inline expressions while their voices are still sounding, matching the Work-backed path.
Otherwise the cancellation cursor can pass those expressions before emergency termination and leave the new channel wave permanently blocked.
Only an unsounded lifetime is marked canceled;
an established lifetime's essential original Off remains eligible when an emergency termination is rejected.

Explicit Reset also discards known canceled input associations, including a standalone CC88 that the host rejected.
At that reached boundary, known prefix state associates later consumers with the required neutral result, while unknown state stays unknown.
The receiver's actual nonzero value remains unchanged until a real repair is accepted;
new input can be captured during rejected repair without copying that old value into its later consumer.
Genuine post-cut CC88 input still overrides later associations.
Reset completion waits for the old lease generation, its exact cancellation cut and every physical/output-retention obligation.
After a new lease is adopted, its genuine post-cut pending note cannot also be required to finish before Reset clears the fault that inhibits that note.
Lease detachment, sealing and transition settlement retain their full old-obligation checks.
Nonessential output for a newly adopted stream waits for current callback coverage and actual Hub admission of its initial zero-cut baseline.
Admission can precede snapshot acknowledgement when calibration places the snapshot ahead of the Hub's playhead;
the pending snapshot still fences transfer of later accepted history, without delaying its physical output.
The existing one-byte adoption state retains this initial admission fact through later gate closure so established controls stay responsive before snapshot acknowledgement;
new offer adoption resets it, and fresh attacks and setup still claim current admission separately.
This prevents a post-Reset controller from being accepted before Adopt, transferred into an unknown Hub row, and then blocking the note behind an unacknowledged nonempty initial baseline.
An obsolete zero-cut baseline acknowledged only to move the join floor keeps that gate closed;
the retained snapshot's original coverage identifies that reply, independent of mutable current coverage.
Established streams retain their responsive controls and essential releases.

## Executed functional coverage

The exported-factory fixtures reach the real 256-credit boundary and normal host acceptance path with the process allocation/deallocation guard enabled.
They cover delayed setup and exact f64 expression/duration, a gate reached before any controller, more than 2,048 retained controls with independent established expression/release, sostenuto edge order, raw bank/program and RPN/NRPN transactions, partial setup rejection and Reset recovery, CC120/123 across waves and owed physical Offs, and neutral idle with retained transaction history.

The Stop fixture first accepts 512 setup events from a 640-control prelude.
Its next callback spends 64 visits capturing Stop and 1,856 capturing 29 complete 64-target groups, leaving too little budget for the 192-visit Stop advance;
setup still makes zero host attempts at or beyond that captured boundary.

CC88 fixtures separately reach healthy prefix output before consumer capture, older-Off reconciliation, delayed consumer restoration, all three prefix/note acceptance outcomes, and rejected repair before an owed MIDI Off.
Review regressions cover accepted unmapped corrections with positive Hub state and acknowledgements, repeated partial-prefix failure, per-setup lease closure before/after claim, and retained raw-pedal repair across Reset.
The cancellation fixture proves all 512 expressions and the final CC88 were captured while visit pressure leaves that prefix unattempted, then requires the real post-Stop consumer and forbids restoration of canceled55. Its negative control retains the necessary inline-cancellation correction while using the previous Stop association implementation.
Additional Stop cases cover unknown startup across two Stops, accepted and rejected neutralization, later original37 overrides, exact deferred-consumer order and complete pin retirement.
Explicit Reset cases cover rejected55 with known zero or unknown state, a captured consumer during rejected nonzero repair, and exact accepted repair0 then post-cut37 before its consumer.
The folded-bend fixture proves its raw history is gone before testing the replayed checkpoint;
the join-floor fixture receives a real Hub retry and proves new controls wait for the replacement zero-cut snapshot.
The far-ahead calibration fixture closes a truthfully joined lease through an ordinary pairing edit while its zero-cut snapshot is still unacknowledged, then verifies exact established expression, controller and Off timing and complete retirement.
The full output fixture executes 512 normal plus 128 emergency attempts and retains all accepted emergency facts.
Wrapper boundary tests verify the independent acceptance masks and exact raw MIDI bytes/flags.
These functional results do not provide a timing or D512 sequencing claim.
The [allocation ledger](adaptive-tuning-aggregation-memory.md) accounts for actual current cells and the separate future reservations.
