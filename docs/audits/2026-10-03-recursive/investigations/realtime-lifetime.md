# A6: configuration, restore and callback payload lifetimes

Investigator: audio_tuning.
Source pinned at `ad1c6e6b9474dd80098703ae15cac241bba0eb41`.
Status: deep for current configuration/restore ownership and allocation-guard reach;
partial for real-host lifecycle scheduling and timing, which were not exercised.
No builds, benchmarks or production edits were performed for this leaf.
**Allocation-instrumentation execution is unverified in this audit run.** Source and fixture reach were inspected deeply; coordinator release tests must not be reported as allocation/deallocation proof.

References below use `W` for `vendor/nice-plug/src/wrapper/clap/wrapper.rs`,
`C` for its sibling `configuration.rs`,
`CA` for `configuration_adapter.rs`,
and `IA` for `input_adapter.rs`.
Plugin paths abbreviate `crates/harmonigraph-plugin/src/`.
This extends the current-source check of [the prior CLAP audit](../../clap-boundary-1415.md), not its original revision's line map.
The separate A1 Hub discarded-capture budget is deliberately outside this leaf.

## Conclusion and engineering assessment

No additional defect was verified in the examined configuration and restore payload lifetimes.
An adjacent source check did identify a concrete avoidable locking representation in the callback's published tail status, detailed below;
its runtime cost and contention are unmeasured.
The design separates heap-owning preparation from fixed-value musical adoption.
That separation earns its place: restoration requires parsing strings, replacing saved documents and serving coherent immediate readback, while audio needs bounded copied commands and retained input values.
Collapsing those paths into a single audio-owned state object would add destructor and locking hazards rather than simplify the actual contract.

The worthwhile actions are accurate verification labeling and consideration of that small tail-status representation simplification, not another queue, deferred-deletion service, global planner or generic realtime framework.
Debug exported-callback tests can establish allocation/deallocation behavior for their reached paths;
release tests and helper-only tests establish different properties.
None of these prove a host callback cannot block or that every legal host lifecycle interleaving has been measured.

## Restore: who owns the heap and where it is released

`ConfigurationEdit`, `ConfigurationCommand` and `ConfigurationSnapshot` are fixed-size `Copy` values (`C:18–57`).
Commands contain five optional f32 values and eighteen i32 payload words, plus origin and ID.
They contain no String, Vec, Box, Arc or trait object.
The queue contains at most 128 such commands (`C:9,184–196`).
Popping, replacing or abandoning an individual command therefore cannot run a heap-owning destructor.
The rtrb storage and mailbox allocation have longer-lived owners; the command itself does not own them.

GUI submission (`C:201–222`) validates numeric values and serializes producers under a producer mutex.
The mutex is released before the wake closure runs.
Audio holds the consumer, publishes fixed atomic words and never acquires that producer mutex.
`PublishedConfiguration::load` can retry (`C:124–150`), but readback/save/preview are its off-audio consumers in this route.
Audio's publication is a fixed word pass, not a seqlock retry loop (`C:90–121`).

Both shipped classes provide `clap_setup` (`plugin/lib.rs:881`; `plugin/tuning/plugin.rs:117`).
`set_state_object_from_gui` selects that route before the generic active-state round trip (`W:1856–1871`).
`restore_setup` prepares an owned object, restores the remaining configuration/state, and commits only after success (`W:1916–1934`).
The production preparation is `Box<Restore(Arc<Shared>, InstanceSettings)>`, where InstanceSettings owns a name String (`plugin/tuning/setup.rs:232–265`).
Commit sets atomic controls and moves the name through `set_name`, whose bounded 80-character collection and old-name destruction occur on this off-audio caller (`setup.rs:176–179,242–247`).
On refusal, dropping the prepared Box, Arc and parsed String also happens on that caller.
Neither the prepared object nor its name enters the configuration ring.

`restore_configuration` (`CA:364–414`) clones the mailbox off audio, parses the musical fields, reads current modulation for preview, removes the audio-owned fields from the generic state object, and deserializes the remaining ordinary/persisted fields.
`ConfigurationMailbox::restore` first checks queue space under the sole producer lock (`C:225–252`).
Only after preparation succeeds does it install the fixed accepted-state shadow, enqueue a Restore command and publish its accepted ID.
Consumer progress can only free that reserved space.
A full queue therefore refuses before changing accepted musical state.
The shadow is one fixed Snapshot, not a retained parsed state graph or one heap allocation per pending restore.

Immediate readback/save intentionally overlays accepted restore intent until audio publishes its applied ID (`C:256–276`; `CA:419–441`).
That is a distinction between accepted and applied ownership, not a second musical reducer.
The real reducer remains `plugin/configuration.rs:255–320`.
It consumes fixed commands and records the resolved value; it does not deserialize saved strings.

Visual/map payloads take their own off-audio path.
`UiState::set` replaces the String under its blob lock and publishes a pending flag (`plugin/editor/persist.rs:205–218`).
The window or closed-editor worker later adopts the visual state.
Saving an open window can wait for the shared editor lock and serialize the UI (`persist.rs:221–243`), but is not callback work.
`AudioMaps::adopt` reads a document using `try_read`, copies only fixed geometry, and uses separate `try_lock` attempts for editor/playback values (`plugin/lattice_maps.rs:230–280`).
The audio bank does not clone map names or replace the heap-owning document.
Contention leaves prior adopted values rather than waiting.
Its history reserves 8192 entries at construction and removes the oldest before pushing at capacity (`lattice_maps.rs:169,201,315–330`).
That source capacity argument is separate from test coverage of the maximum history.

The inherited generic fallback remains allocation-permitted and potentially blocking.
`set_state_inner` uses `permit_alloc` around deserialization/reinitialization and acquires the plugin mutex (`W:2006–2054`).
The active generic GUI path sends the state to audio and receives it back for off-thread destruction (`W:1873–1902`).
Those facts are not a shipped-path defect: both local classes take setup restoration before that fallback.
Conversely, a passing opt-in fixture must not be generalized to every third-party plugin using this vendored wrapper.

## Bounds and cancellation

#1417 / `b548373c` already fixed the configuration command-drain mechanism identified by the prior audit.
Current `drain_configuration_commands` snapshots `commands.slots()` and visits only that prefix (`CA:202–222`).
Its upper bound is 128 attempted commands at entry.
An apply refusal retains the front command and returns.
Concurrent replenishment cannot extend that loop's current iteration count.
The sixteen notification cells bound outstanding UI/Learning notification transactions, but Restore bypasses those cells;
the entry snapshot, not the notification bank, supplies the restore-work bound.
Do not report the old unbounded loop as a current finding.

The shared owned-input pool allocates 2048 cells once (`C:337–346`) and stores copied input records.
`InputValue`/`OwnedInput` are fixed Copy values, including copied transport fields; unsupported payloads are not retained host pointers (`C:281–335`).
Capacity is checked before insertion (`C:354–360`).
The configuration, performance and ordinary walker cursors retain input until every enabled consumer has acknowledged it.
`IA:37–44` reclaims only their minimum completed prefix.
Configuration refusal therefore cannot be bypassed by performance consumption.
These cursors are independent readers of one owned input pool, not duplicate event queues to combine without preserving their acknowledgment contract.

`reset_timed` loops exactly the old pool length, removes timed records and reinserts only untimed flush records (`C:375–382`).
`IA:46–54` resets cursor offsets after this explicit cancellation.
This can destroy at most 2048 fixed records; it does not deallocate the pool.
It is cancellation, not a promise that every cancelled event was performed.
Resetting callback fault state also does not silently discard all accepted configuration commands;
the production full-command fixture below checks eventual adoption after reset.

These are hard iteration/storage bounds, not measured execution deadlines.
The plugin's reduction hooks and host output calls still contribute work within each attempt.
Wrapper mutexes do exist, and plugin musical-output hooks can hold the plugin mutex across host `try_push` calls.
The inspected local ownership relies on serialized same-instance audio callbacks and main lifecycle exclusion, as stated in the prior audit.
The allocation guard cannot validate those host scheduling obligations or the time a host spends in its callback.

## Shutdown and final owners

The exported `destroy` first retires configuration, then calls the plugin main-destroy hook, then releases the wrapper (`W:2096–2107`).
`retire_configuration` takes the runtime out of its mutex, checks pending commands/owned input, tells the plugin whether work was unfinished, and drops the original runtime off audio (`CA:41–50`).
It does not simulate a successful application cut for abandoned work.
Mailbox handles held elsewhere can keep the mailbox/ring producer alive, but their wake closure holds only a Weak wrapper reference (`CA:55–69`).
Once destruction completes, such a handle cannot resurrect the wrapper through that closure.
Safe lifecycle teardown still presumes callers do not concurrently destroy an instance whose methods are in use.

The Hub main-destroy hook takes the original Hub, configuration Owner and Recorder, proves its final frontier while rings are still readable, retires the endpoints and transfers publication ownership to the existing retirement path (`plugin/lib.rs:911–918`).
This leaf checked the handoff order, not the complete recorder-finalization protocol covered by the recording investigation.
Tune's retirement returns row endpoints on the main thread (`plugin/tuning/tune.rs:190–196`; `session.rs:298–305`).
An audio-side row claim only takes an existing endpoint through a bounded 16-row `try_lock` scan (`session.rs:278–294`), rather than allocating/rebuilding its rings.

`Harmonigraph::drop` requests graphics/export worker shutdown off audio (`plugin/lib.rs:128–136`).
Its background analyzer field precedes the editor state it uses (`lib.rs:102–120`).
`BackgroundAnalyzer::drop` sets a stop flag and joins (`plugin/background.rs:154–165`);
the worker performs a final owned display drain after callback quiescence (`background.rs:337–347`).
This allows blocking/destruction where lifecycle owns it instead of adding an audio garbage-collection queue.
The join can wait for the current tick/restore, sleep and final drain.
It is not a strict numerical shutdown deadline, and the main lifecycle is intentionally outside callback allocation guards.

## What allocation instrumentation actually establishes

`vendor/nice-plug/src/wrapper/util.rs:206–216` wraps the complete process closure with `assert_no_alloc` only when **both** `debug_assertions` and `assert_process_allocs` are enabled.
The global allocation guards at `util.rs:50–56` are also debug-assertions gated.
Enabling `clap-boundary-tests` without `assert_process_allocs` is a compile-time error (`util.rs:37–47`).
Together, they install MeasuredAllocator backed by AllocDisabler, not an unguarded counter.
`wrapper/allocation_probe.rs:35–54` forwards allocation, zeroed allocation, reallocation and deallocation through that base allocator.
Inspection of the resolved `nice-assert-no-alloc 1.0.0` implementation confirms the same forbid check on both `alloc` and `dealloc`.
Thus an unexpected final Arc/Box/String free on a guarded callback is relevant, not merely fresh allocation.

The boundary is thread-local and limited to execution inside a guard.
The optional measurement helper explicitly excludes other threads and allocator-private overhead (`allocation_probe.rs:58–69`).
It is not RSS, a blocking detector or a callback timer.
Release-profile test execution skips the wrapper guard and debug-only global allocator even when both Cargo features are selected.
Release tests therefore provide functional evidence, not fresh allocation/deallocation evidence.
The explicit publication helper's direct `assert_no_alloc` call likewise cannot turn a release binary without the guarded global allocator into allocation coverage.

Current CI has the necessary debug commands: `ci.sh:127` runs the workspace tests with `nice-plug/assert_process_allocs`, and `ci.sh:280–282` separately runs vendor `clap_boundary` and `clap_auxiliary` with both features.
The production plugin also selects both features in its dev-dependency (`crates/harmonigraph-plugin/Cargo.toml`).
The workspace command does not execute the separate vendor workspace's integration tests;
the explicit vendor command does not execute every vendor unit/macro test.
This audit inspected that configuration but did not rerun those debug commands or claim their current results.
Exact existing debug commands for continuation, if new evidence justifies their build cost:

```sh
cargo test --workspace --exclude harmonigraph-render --features nice-plug/assert_process_allocs
cargo test --manifest-path vendor/nice-plug/Cargo.toml \
  --target-dir target/debug/vendor-nice-plug \
  --features assert_process_allocs,clap-boundary-tests \
  --test clap_boundary --test clap_auxiliary
```

`W:2252` guards the complete exported process body, including capture, configuration, plugin hooks and output drains.
Start/reset/stop use guards around their audio-role work (`W:2199–2234`).
Opted-in `params.flush` is guarded (`W:3444–3465`), while the legacy opt-out flush branch is not.
Construction, activation, state parsing, main service, deactivate and destroy are intentionally outside these callback guards.

## Fixture reach: production, wrapper and helpers

Production configuration tests construct the real exported Harmonigraph factory, call init/activate/start, invoke process/flush/state vtables and stop/deactivate/destroy on fixture drop (`plugin/configuration/tests.rs:1,169–215,330–425`).
Their host output sink reserves observation vectors before process and asserts capacity before pushing (`tests.rs:120–153,340–350`).
This prevents fixture growth from masquerading as plugin callback allocation.
It also means these fixtures exercise a controlled in-memory host, not Bitwig or an actual GUI window.

| Fixture | Path and reached behavior | Limit |
| --- | --- | --- |
| `active_restore_without_callbacks_has_coherent_save_readback_and_ordered_adoption` (`plugin/configuration/tests.rs:459`) | Real active Hub, both host-state and GUI-state entry, restore → UI edit → newer restore, immediate save/readback, then callback adoption | GUI route is the wrapper API; no native editor opens. |
| `restore_preserves_held_modulation_without_a_new_host_mod_event` (`:761`) | Real modulation event followed by restore, snapshot comparison and saved unmodulated value | One parameter/modulation scenario, not all host mod sources. |
| `a_full_command_queue_keeps_accepted_state` (`:789`) | Two restores plus 126 UI commands fill all 128 slots, next submission refuses, accepted save remains coherent, reset and real callbacks eventually apply accepted command ID | Does not impose adversarial concurrent refill. |
| `one_owned_input_pool_reaches_2048_in_a_callback_and_refuses_growth_past_it` (`:832`) | Real process reaches 2048 events, 2049 refuses, exported reset recovers | Fixed 64-frame callback and selected parameter events. |
| `destroying_pending_or_faulted_configuration_owners_does_not_panic` (`:856`) | Both a pending accepted configuration and an input-capacity fault survive into real device teardown | Destruction is outside audio guards; the assertion is successful teardown, not a heap/timing measurement. |
| `lattice_maps_restore_without_editor_preserves_geometry_and_shared_tuning` (`:2260`) | Named heap-owning map document restored with no editor, preview and audio geometry adoption checked | Does not fill all 8192 map-history entries under an allocation guard. |
| `restore_arriving_during_configuration_drain_waits_for_next_callback` (`vendor/nice-plug/tests/clap_boundary.rs:1510`) | Synthetic exported combined fixture pauses the real apply hook, restores concurrently, verifies published 0.25 versus accepted 0.75 after callback one and apply samples `[0,64]` after callback two | Deterministic finite interleaving, not an infinite producer or timing benchmark; hooks are synthetic. |
| `owned_pool_waits_for_configuration_then_reclaims_the_complete_prefix` (`:1551`) | Apply refusal retains all 2048 records; extra event and callback transport refuse; releasing refusal delivers original identities once; another 2048 fill proves physical reuse | Synthetic plugin observers, real wrapper/pool. |
| `reset_cancels_faulted_timed_input_and_retains_untimed_flush_once` (`:1604`) | A valid note prefix reaches observers before a NaN parameter faults; untimed flush survives exported reset with provenance; another full pool succeeds | Cancellation/ownership evidence, not a production NaN-audio test. |
| `canonical_publication_slots_and_loss_are_allocation_free` (`plugin/configuration/tests.rs:2086`) | Explicit helper guard fills every snapshot slot with 64-voice payloads, reaches Busy/Lost, drains off guard, reuses payload storage under guard | Not an exported callback; guards the publication helper operations it invokes. |
| `the_thread_runs_on_its_own_and_stops_when_dropped` (`plugin/background.rs:940`) | Real worker consumes audio, is dropped/joined, then later queued audio causes no new columns | Separate worker harness; its generous two-second check is not a real-host destructor deadline. |

The synthetic vendor fixture deliberately uses preinitialized mutex-protected observation vectors and barrier-like atomic spin points (`clap_boundary.rs:220–269,567`).
Passing it does not prove a production callback is lock-free or never yields.
Its value is exact ordering/refusal/reclamation reach through the real wrapper.
The auxiliary fixture exercises buffer layouts and present/missing auxiliary buffers, not arbitrary musical configuration.
Direct Owner/map tests with no explicit guard supply behavior or capacity evidence only.

## Adjacent representation finding: only a tail length is read

Status: source-confirmed mechanism and coordinator-confirmed native size/lock-free query; contention timing was not executed.
The check was prompted by another project's analogous candidate, then independently traced against this repository and its locked dependency sources.
No result from that other project is used as evidence here.

`W:180` stores `last_process_status: AtomicCell<ProcessStatus>`.
The wrapper stores every processed subblock's result at `W:2543`, and resets it to Normal at start (`W:2194`).
The only read of that field in this CLAP wrapper is `ext_tail_get` (`W:3633–3637`), which maps Tail(samples) to samples, KeepAlive to `u32::MAX`, and every other status to zero.
The original local `result` already supplies CLAP's immediate result/error handling (`W:2549–2557`).
No consumer needs the stored error message or enum discriminant beyond that tail mapping.

The resolved `nice-plug-core 0.1.5` definition (`src/plugin.rs:263–275`) includes `ProcessStatus::Error(&'static str)`.
On a 64-bit target, that variant requires at least the two-word string-slice payload, regardless of which small variant the live plugin happens to return.
`Cargo.lock:661–665` pins crossbeam-utils 0.8.22.
Its `src/atomic/atomic_cell.rs:353–381` chooses primitive atomics only for matching unit/u8/u16/u32/u64 layouts; the u128 option is commented out.
The larger ProcessStatus therefore selects its fallback.
`atomic_store` (`atomic_cell.rs:1122–1134`) takes `lock(address).write()` before copying the value.
That lock is selected from 67 global address-hashed SeqLocks (`:1012–1040`).
`src/atomic/seq_lock.rs:47–62` loops with backoff while the selected lock is held.
Loads first attempt an optimistic read and can also take that write lock (`atomic_cell.rs:1080–1114`).

This establishes a real lock-taking operation on the ordinary callback path, even when the plugin returns only Normal or KeepAlive.
It does not establish an observed stall, audible dropout, typical contention, or a measurable CPU saving.
Another unrelated large AtomicCell can share a hashed lock, but no collision was measured in this application.
The field predates the local opt-in work: blame attributes it to `8e308a9744`, “Measure Bitwig central-tuning delay and fix CLAP activation latency” (#630).
The prior CLAP audit already cautioned that its AtomicCells were not all lock-free; this narrows that caution to one removable case.

Candidate: publish only `AtomicU32 last_tail_samples`, with the existing Tail/KeepAlive/other mapping applied when the local result is stored and zero at start.
Keep the original ProcessStatus local for immediate CLAP status/error handling.
This removes the unnecessary full-status cross-thread representation and its fallback lock without introducing a new ownership protocol.
Upfront cost is one small wrapper edit and a focused exported-tail fixture for Normal/Error/Tail/KeepAlive plus start reset.
The added maintenance burden is the existing four-case tail mapping moving to the writer; no new cache invalidation or persistent state is needed.
No inspected current vendor/production fixture calls the CLAP tail extension to establish those mappings.
An allocation-guarded process test cannot detect this lock because acquiring it need not allocate.

Verdict: pursue the narrow representation simplification after confirming the current target's `AtomicCell::<ProcessStatus>::is_lock_free()` and tail behavior if implementation is authorized.
Do not sell it as a measured performance optimization.
Other large AtomicCells (`current_audio_io_layout`, `current_buffer_config`) remain in the wrapper, so this change alone would not establish a lock-free callback.
Their ownership and update cadence differ and were not benchmarked or redesigned here.

## Proportional next action

Verdict: retain the current configuration/restore ownership split and #1417 snapshot bound.
For those lifetimes, no new mechanism or permanent fixture is proposed.
The separate tail-status candidate above is a small representation change with a specific missing exported-tail behavior test.
When reporting coordinator results, label release functional tests separately from debug allocation-guard execution.
If fresh callback allocation evidence is required, use the existing debug commands or the narrowly named exported fixtures above rather than writing another helper-only test.
Real-host callback timing, thread migration, native window teardown and hostile/reentrant host behavior remain outside the observed execution scope.
Those are explicit limits, not demonstrated product failures or reasons to impose a broader framework.

## Coordinator native confirmation

The real linked dependencies report ProcessStatus size 24, alignment 8 and AtomicCell lock_free=false; u32 is size/alignment 4 and lock_free=true.
The native query does not measure contention or callback time.
The independent challenge additionally records per-subblock, early-fault retention and start-only clearing requirements.
See logs/probe-atomic-status.log, experiments/atomic_status_probe.rs and investigations/challenge.md.
