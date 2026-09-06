# Bounded native GUI parameter delivery

This framework stage extends the [owned CLAP performance boundary](adaptive-tuning-clap-performance.md).
It delivers native `GuiContext` parameter edits to the same retained input stream as host automation,
without depending on host echo or accepted outbound notifications.
Production Source participation/Birth integration remains the central sequencing stage's verification obligation;
this stage does not complete adaptive tuning or establish Bitwig timing.

## Ownership and ordering

The existing 2,048-entry GUI notification bank now uses rtrb.
Main-thread raw Begin/Set/End calls serialize its producer with a mutex;
the consumer exposes only an exclusive,
atomically checked borrow and adds no audio-side mutex.
The consumer's `Sync` implementation is justified by that exclusive interface:
it never exposes `AtomicRefCell::borrow`,
and a concurrent consumer attempt fails instead of waiting or sharing rtrb's interior counters.
All queue borrows end before host callbacks.

One head-relative admission count distinguishes entries already copied locally from notifications still awaiting host acceptance.
A refused Begin can retain the bank's head while later Sets enter input once.
Accepted notifications pop that head and decrement the count;
they never reapply a value or append input.
No notification cursor participates in `InputStorage` reclamation.

Each top-level process/flush capture takes one finite GUI snapshot before invoking host input callbacks.
It reserves the complete accepted host batch and enclosing transport first,
then charges every inspected GUI entry,
including gestures,
against the remaining 2,048-entry input-work allowance and free input cells.
No capacity or work leaves the unadmitted prefix retained.
Invalid host input rolls back the entire tentative batch and leaves GUI admission unchanged.

Process order is older retained input,
enclosing transport,
admitted GUI Sets at enclosing offset zero,
then new host events.
Flush Sets remain untimed until the existing next-process binding path.
GUI arrivals after the snapshot wait for the next top-level capture even when the current callback traverses configuration subblocks or exits with an error.
Configuration-owned parameters follow the configuration reducer;
generic parameters follow the existing owned input walker exactly once.
The configuration-only and legacy wrappers retain their existing direct parameter-output path.
No persisted shape or public parameter units change.

## Verification and storage

The feature-gated wrapper helper constructs the real `make_gui_context` and resolves its real `ParamPtr`.
Six guarded exported-factory fixtures call raw Begin/Set/End through that context.
They cover missing output and refused Begin/Set retries with newer host automation,
a fully occupied 2,048-cell retained input pool,
gesture budget exhaustion,
invalid-batch rollback and untimed flush,
post-snapshot arrival across combined configuration subblocks and both error exits,
physical wrap of a full notification bank,
and both nonperformance paths.
The wrap fixture fills all 2,048 notifications,
retires 512,
then refills those 512 cells while the other 1,536 remain admitted and pending.
The allocation/deallocation guard stays enabled throughout real process/flush callbacks.

On macOS arm64,
`OutputParamEvent` remains 16 bytes and `Option<OwnedInput>` remains 176 bytes.
The original queue requests 49,152 bytes in one allocation;
rtrb requests 33,280 bytes in two allocations,
including its 32,768-byte payload and 512-byte Arc/header allocation.
Its producer and consumer are 24 bytes each;
the main-side mutex owner is 32 bytes,
and the exclusive consumer plus admission count is 40 bytes.
The original inline queue owner was 384 bytes and its separate pending-event mutex was 24 bytes.
Compiling the exact old and new wrapper field definitions with the actual plugin types measures Harmonigraph at 9,216 → 8,960 bytes and Tune at 8,704 → 8,448 bytes,
both aligned to 128 bytes.
Including that padding,
the change reduces allocated storage by 16,128 bytes per wrapper,
or 274,176 bytes across one Harmonigraph and sixteen Tune instances,
while adding one allocation per instance.
Compile-time ceilings retain the 16-byte event,
32-byte producer owner,
40-byte exclusive consumer owner and 384-byte ring header;
the existing 192-byte owned-input ceiling and both 2,048-entry capacities remain unchanged.
Allocation requests exclude allocator-private overhead and RSS.

The rejected alternatives were a second GUI payload bank,
one pending notification cell,
admission during output drain,
and reapplying values during notification retries.
The single pending cell cannot reach a Set behind a rejected Begin;
drain-time admission leaks later GUI edits into the current configuration walk;
retry-time mutation overwrites newer host automation.
