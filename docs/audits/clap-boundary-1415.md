# CLAP boundary audit — #1415

Audited 2026-10-02 against `59467b4751b784227353db3668df8c962aa69349`.
Scope and requested deliverables: [#1415](https://github.com/yan-h/harmonigraph/issues/1415).
Line numbers below describe that base revision;
the regression names describe the accompanying change.
This is a source and exported-factory audit, not a Bitwig scheduling measurement.

## Finding and fix

**The configuration command drain had no finite callback budget.** `vendor/nice-plug/src/wrapper/clap/configuration_adapter.rs:210` repeatedly peeked and popped until the producer was empty.
A concurrent producer could refill freed slots throughout the callback.
The 16 notification cells limit UI/Learning commands, but Restore commands bypass them;
the production `Owner::apply` (`crates/harmonigraph-plugin/src/configuration.rs:276`) has no command-count limit either.
Thus the ring's 128-cell capacity bounded memory but not callback work.

The drain now snapshots `commands.slots()` once and attempts only that prefix, in FIFO order.
An apply refusal still retains the current command and returns immediately.
A command arriving after the snapshot stays pending for the next callback;
accepted restore readback/save still comes from the mailbox shadow immediately.
No persisted shape or musical-value conversion changes.

`restore_arriving_during_configuration_drain_waits_for_next_callback` pauses inside the first real apply hook and restores a second state from the main thread.
On the original loop, the published value was already `0.75` instead of the expected first restore's `0.25` when the first callback returned.
With the fix, it remains `0.25`, visible accepted state is `0.75`, and the recorded application samples are exactly `[0, 64]` after the next callback.
This reproduces the mechanism that allowed replenishment without an unbounded timing test.

No additional in-scope defect was verified.
The performance adapter's module comment also overstated lock release across host calls;
it now distinguishes wrapper parameter notification attempts from plugin musical-output hooks.

## Lock table

`W` denotes [wrapper.rs](../../vendor/nice-plug/src/wrapper/clap/wrapper.rs),
`CA` [configuration_adapter.rs](../../vendor/nice-plug/src/wrapper/clap/configuration_adapter.rs),
`IA` [input_adapter.rs](../../vendor/nice-plug/src/wrapper/clap/input_adapter.rs),
`PA` [performance_adapter.rs](../../vendor/nice-plug/src/wrapper/clap/performance_adapter.rs),
and `C` [configuration.rs](../../vendor/nice-plug/src/wrapper/clap/configuration.rs).
An arrow means guards are held together, not merely acquired in that order at different times.
`AtomicRefCell` borrow conflicts panic rather than wait;
shared immutable borrows may overlap.
There is no `RwLock` field on `Wrapper` itself.

The thread argument assumes the CLAP contract:
`init`/`activate`/`deactivate`/`destroy` are main-thread lifecycle operations,
`start_processing`/`stop_processing`/`reset`/`process` use the serialized audio role,
and `params.flush` is audio-thread when active and main-thread when inactive, never concurrent with `process`.
A host's audio role may migrate between OS threads.
Off-thread state restoration and GUI submission can overlap processing.

| Lock or borrow | Callbacks / threads and acquisition sites | Nested order and whether audio can wait |
| --- | --- | --- |
| `plugin: Mutex<P>` | Main construction/install (W:869, CA:78), init/activate/deactivate/destroy (W:2093–2176), configuration retirement (CA:48); audio start/stop/reset (W:2193–2226), process and opted-in flush (W:2284,2518; CA:250; IA:64,227; PA:19,38,52); inactive `on_main_thread` (W:2660) | Audio uses input → configuration → plugin, input → plugin, or buffer manager → plugin → event borrows. Main lifecycle excludes same-instance processing. Neither shipped plugin's restore takes this mutex. |
| `plugin`, inherited state fallback | `set_state_inner` reinitializes under this mutex (W:2038); GUI fallback can enter it at process tail (W:2588) | Can block generic-plugin audio against main state load; initialization/deserialization also deliberately permit allocations. Hub and Tune use setup restoration instead (W:1916), so the local paths do not depend on this fallback. |
| `configuration: Mutex<Option<Runtime>>` | Audio process/reset and active flush: CA:220,230,246,490,499,588; main install/retire: CA:42,79 | Input → configuration → plugin during reduction. Notification guards end before host `try_push`; install/retire do not hold configuration across plugin acquisition. No legal concurrent main/audio taker. |
| `owned_input: Mutex<Option<Runtime>>` | IA:70,81,98,124,141,155,196,204,217,225,250,279; CA:45,244; W:2194,2221 | Process/reset/flush own it serially; main retirement follows joined callbacks. GUI producers do not touch it. No cross-thread wait in the shipped path. |
| GUI consumer `output_parameter_events: AtomicRefCell` | Process capture/drain, serialized flush; IA:95,153,209; PA:102,112; W:1109 | Consumer → input in admission and input → consumer at commit are both on the same serialized role; no realizable two-thread cycle. Host notification calls run after releasing the consumer. |
| GUI producer `output_parameter_sender: Mutex` | GUI context parameter setters enqueue at W:953 | Separate rtrb producer from the audio consumer. Producer guard ends before host flush request. Audio never takes it. |
| Mailbox `producer: Mutex` | UI submit, host/GUI restore, readback/save via `visible`; C:205,232,256 | Producer → off-thread state preparation/preview in restore. Audio only reads the ring consumer and publishes atomics; it never calls `visible`. |
| `host_params: AtomicRefCell` | Init writes W:2078; GUI setter W:956, task/readback W:512,2662; mailbox wake CA:60; configuration main callback CA:332 | All post-init access is shared. The mailbox wake copies the pointer and ends the borrow before `request_flush`; no waiting lock or producer guard crosses it. |
| `host_state`, `host_gui`, `host_latency`, `host_voice_info`, `host_thread_check`: `AtomicRefCell`s | Init writes W:2074–2089; shared reads in thread checks, queued tasks, latency publication, GUI resize, state notification (W:439,482,503,973,1966,2665; CA:334) | Init cannot overlap processing. Post-init shared reads may overlap; no mutable borrow is acquired by these callbacks. |
| `buffer_manager: AtomicRefCell` | Main activate W:2143; audio process W:2322 | Buffer manager → plugin while executing each subblock. Activation excludes processing; mutable borrow ends before wrapper output drain. |
| `input_events`, `output_events`: `AtomicRefCell<VecDeque>` | Process contexts W:929–930, legacy flush W:1071,1181 | Buffer manager → plugin → both event cells during process context; serialized flush borrows them directly. No GUI producer accesses these queues. |
| `editor: AtomicRefCell<Option<Mutex<Editor>>>` | Main GUI extension methods and main queued parameter/editor tasks, W:457–478,974,3010,3041,3058,3071,3118,3157 | Editor cell → editor mutex. Performance callbacks defer GUI tasks; no shipped audio path acquires either. |
| `editor_handle: Mutex` | Main GUI create/destroy, set-parent and task checks, W:456,463,473,2061,2971,2986,3135 | Set-parent nests handle → editor cell → editor mutex. No shipped audio access. |
| `task_executor: Mutex` | Main/background tasks W:454; initialization context `context.rs:88` | Initialization can hold plugin → executor, before processing. Audio schedules work instead of executing the closure. |
| `background_thread: AtomicRefCell` | Construction W:863; shared scheduling W:430 | Immutable borrows after construction; scheduling uses the background queue. |
| `this`, `clap_plugin`: `AtomicRefCell`s | Construction/vtable setup W:547,849,860; GUI-context/test setup reads | No processing mutation or competing exclusive borrower. |
| Debug `ClapGuiContext::param_gesture_checker: AtomicRefCell` | GUI begin/set/end (`context.rs:171,208,238`) | GUI-only debug bookkeeping; no audio access. |

The wrapper's crossbeam `AtomicCell` fields are not all necessarily lock-free for their payload sizes.
Layout/buffer configuration writes occur outside processing;
process mode may be atomically updated by the host, and process status is atomically read for tail queries.
These inherited primitive operations are not a proof of a lock-free wrapper;
this audit establishes the local ownership and absence of long overlapping critical sections, not a platform-specific lock-free guarantee.

### Plugin-side shared owners and hooks

Paths here are under `crates/harmonigraph-plugin/src/`.

| Lock | Thread / hook and sites | Can audio block? |
| --- | --- | --- |
| Instance registry mutex | Main `clap_main_init`/`clap_main_destroy`, setup/editor snapshot/edit; `tuning/instances.rs:18,22,27,75` | No audio accessor. Main snapshots nest registry → shared name. |
| Shared name mutex | Setup naming/save/snapshot; `tuning/setup.rs:155,166,255`, `tuning/instances.rs:37` | Main/editor only; audio uses atomic control fields. |
| Session row endpoint mutexes | Blocking Hub attach/detach and Tune retirement (`tuning/session.rs:257,265,298`) under main lifecycle plugin lock; Tune audio claim uses `try_lock` at :278 | Other instances may process during these main operations, but their audio claims skip a contended row in a bounded 16-row scan. They do not wait. |
| Map document `RwLock`, editor and playback mutexes | Configuration adoption uses `try_read`/`try_lock` in `lattice_maps.rs:245,252,276`; UI accesses at :46–51,106–146 | Real cross-thread contention is possible; audio keeps its prior adopted value when a try operation fails. Each audio guard ends before acquiring the next. |
| Diagnostics logged mutex | `tuning/diagnostics.rs:184`, setup service on main | Audio publishes atomic diagnostics only. |

All local configuration hooks were traced:
install/retire are main lifecycle;
prepare/preview/save are off-thread;
begin/adopt/segment/apply/observe/group-end/fault run under the audio reduction or process path.
The latter reach bounded owner/map/learning/recording work, atomic/ring publication, and the nonblocking map acquisitions above.
All performance hooks were traced:
main init/activate/deactivate/inactive/destroy own registration and lifecycle transfer;
start/stop/reset/begin/input/input-boundary/process/finalize/end stay on the serialized audio role.
Tune's endpoint retry uses `try_lock`, and Hub input-boundary owns sequencing.

The configuration mailbox's submit and restore release the producer mutex before calling their wake closure (C:218,250).
The closure upgrades a weak wrapper reference, copies the shared host-params pointer, requests flush, then requests main-thread service if dirty.
GUI musical edits use that mailbox;
GUI ordinary parameters use their separate producer ring;
GUI state restoration enters the same prepared setup route as host restoration.
Neither shipped class calls the generic synchronous GUI-state round-trip from this route.

Plugin performance begin/process/finalize **do** hold `plugin` across musical output's host `try_push` calls (PA:19,38; W:2518–2533).
Wrapper-owned parameter notifications release their runtime/input/consumer guards before host calls.
No deadlock was demonstrated for legal CLAP reentry:
process/reset/flush cannot reenter each other, and shipped readback/restoration does not acquire `plugin`.
Allocation checks cannot validate a host callback's scheduling or execution time.

## Input-pool reuse argument

Write `L` for storage length and `C`, `P`, `W` for configured, performed and walked offsets from its head.
During normal three-consumer operation:

- `0 <= P <= C <= L` and `0 <= W <= L`.
- Configuration advances only after fetching a live record and successfully applying/observing it (CA:272–310). A refusal or fault leaves that record unacknowledged.
- Performance advances only while `P < C` (IA:228–231), after handing an owned copy to the hook.
- The walker advances only after fetching a live record (IA:252–281). Returning a future transport/automation split leaves its record in place for the next subblock.
- Reclaim pops exactly `min(C, P, W)` cells and subtracts that count from every cursor (IA:37–44), preserving the inequalities. No consumer still needs a popped cell.
- Storage checks `L < INPUT_SCAN` before writing at `(head + L) % INPUT_SCAN` (C:346–358), outside the live interval. Returned inputs are copies, with no retained host pointers.
- Failed capture truncates only the new tail back to `original_len` (IA:202–211). No consumer advances during capture; old records and cursor offsets remain intact.
- Valid-boundary process errors run `finish_owned_walk` before finalization (W:2286,2578). Invalid boundaries do not consume the pool. Configuration failure cannot be bypassed by performance delivery, even when the ordinary walker advances.
- Configuration-only operation pops after observation because the other two owned-pool consumers are disabled. Performance-only operation sets `C = L` before delivery.

**Reset is explicit cancellation, not three-consumer acknowledgment.** `reset_timed` (C:375–382) disposes of timed records, including work a fault prevented configuration/performance from consuming.
It iterates the original length, pops each old entry and pushes back only entries with no bound sample.
This compacts still-untimed flush entries in FIFO order without overwriting live storage;
then `Runtime::reset` zeros all cursors.
Start/reset and mailbox reset-generation adoption use this same operation.
An already bound flush record counts as timed work and is cancelled too.
Unreported capture loss remains latched across reset.

The new `owned_pool_waits_for_configuration_then_reclaims_the_complete_prefix` blocks the first owned parameter, fills all 2,048 cells, and asserts both observers are still empty.
An extra event fails with `Full`.
After releasing configuration, even a callback transport fails capture while the retained batch makes progress;
every original event reaches both observers once, with its original index, note ID and sample.
Another full batch then succeeds, proving capacity reclamation and physical ring reuse.

The new `reset_cancels_faulted_timed_input_and_retains_untimed_flush_once` reaches a nonfinite owned-parameter fault after a valid note prefix.
It appends untimed flush input, invokes the exported reset callback, verifies only that flush survives with its provenance, and fills the pool again.
Existing `owned_input_has_exact_subblocks_transport_and_no_duplicate_consumer` covers walker lag across splits at 16 and 32;
`retained_flush_parameter_is_applied_before_error_recovery_acknowledges_it` covers ordinary parameter application on a process-error exit.

## Allocation-check reach map

CI runs these debug-profile commands:

```sh
cargo test --workspace --exclude harmonigraph-render --features nice-plug/assert_process_allocs
cargo test --manifest-path vendor/nice-plug/Cargo.toml \
  --target-dir target/debug/vendor-nice-plug \
  --features assert_process_allocs,clap-boundary-tests \
  --test clap_boundary --test clap_auxiliary
```

`process_wrapper` guards complete callback execution, not merely the plugin's process method.
It only enables the allocation assertion with both the feature and debug assertions.
CI's release `cargo check` is compilation coverage, not execution or allocation coverage.
The workspace command does not run excluded vendor tests;
the explicit vendor command does not run vendor unit tests or `derive_params`/`derive_persist` tests.
Direct core/Owner/map unit tests outside an explicit allocation guard do not establish callback allocation safety.

| Hook or branch | Executed under the guard | Limits / unreached branches |
| --- | --- | --- |
| Start → performance reset/start and `Plugin::reset` | Every exported production/vendor device starts; explicit reset fixtures include `reset_cuts_and_audio_resumes_within_two_callbacks` and `a_host_reset_on_one_tune_cuts_it_out_of_the_hubs_context` | Production performance-start is a default no-op. Generic initialization is outside this guard. |
| Stop → performance stop | Production teardown/reactivation and stopped-take lifecycle fixtures | Main deactivate/destroy and resource retirement intentionally allocate/deallocate outside audio. |
| Performance begin/input/input-boundary/process/finalize/end | Exported Hub/Tune fixtures exercise paired/unpaired operation, delay/reply queues, output refusal, reset cuts, publication/recording, and bounded overflow | Tune input-boundary is a default no-op. Production helpers always provide an output callback. |
| Missing output / process error | Vendor `missing_output_and_error_exits_finalize_with_a_blind_writer`, unsupported-input and GUI error-exit fixtures | Production host-refusal tests reach the same Tune retain-on-failed-emit path; null output is not independently run with the production plugin. |
| Configuration begin/adopt/segment/apply/observe/group-end/fault | Production `active_restore_without_callbacks_has_coherent_save_readback_and_ordered_adoption`, learning, modulation, UI/flush, recording/map and input/command-exhaustion fixtures | New restore-arrival fixture reaches concurrent replenishment inside apply. New pool fixtures reach refusal/fault before acknowledgment and reset cancellation. |
| Opted-in `params.flush` | Production configuration fixtures; vendor flush-loss, GUI admission and retained-flush/error fixtures | Legacy opt-out flush is not guarded by `process_wrapper`. Flush capture faults reach `clap_configuration_fault`; successful flush waits for a process clock before adoption. |
| Subblocks, transport and parameter output | Vendor exact-subblock/transport, host-refusal, notification gesture debt/reuse and GUI bank-wrap fixtures | Existing tests prove output order and acceptance separately from allocation assertions. |
| Audio buffers and ordinary `Plugin::process` | `clap_auxiliary` exercises present/missing auxiliary input/output and changing frame lengths | Its lack of musical input assertions does not establish legacy process ingress support. |
| Invalid capture | Production negative steady time and capacity exhaustion; vendor scan limit, unsupported data and invalid GUI-host capture | Zero frames, over-maximum frames, timestamp addition overflow and exhausted counters are not independently established by the inspected fixtures. They converge on fixed-value failure/finalization paths; no heap-growth lead found. |
| Adaptive context / map history | Production adaptive held-limit and 304-chord journey; direct map-history capacity tests | The 256-confirmed-pitch fixture disables retuning, so it is not evidence of 256 adaptive context entries. The 8,192 map-history limit is not established by an allocation-guarded capacity fixture. |
| Install/prepare/preview/save and main lifecycle hooks | Outside audio guard by design | Serialization, construction, registration and retirement allocate. Generic fallback restore also uses explicit `permit_alloc`; neither shipped setup path enters it. |

The adaptive working/context vectors reserve 280 entries for at most 256 held or 24 released entries;
candidates reserve 4,096 and check the ceiling before each insertion.
Map history pops at its fixed limit before pushing.
These source bounds supply the missing capacity argument;
no additional capacity fixture was added merely to increase counts without an allocating/blocking branch to test.
The new tests target a proven unbounded loop and previously untested delayed acknowledgment/cancellation paths.
Vendor fixture observation mutexes and deliberate pause/yield loops pass the allocation guard, which demonstrates why that guard is not a blocking detector.

### Known exclusions

The inherited generic state-load mutex/initialization behavior remains excluded and does not occur in either shipped setup path.
The removed legacy process-input walker is also intentional:
`2c1feb1a` ([#753](https://github.com/yan-h/harmonigraph/pull/753), implementing #712/#640) removed it because both shipped classes enable performance mode.
A scratch exported-factory check confirmed that nonperformance process callbacks do not consume legacy MIDI or ordinary host parameters;
configuration-only owned parameters still reduce.
This is not a new defect or evidence of preserved opt-out ingress support.
No baseview, egui-baseview or wgpu-hal audit was attempted.

## Validation

The new restore-arrival regression was observed failing before the fix and passing afterward.
The allocation-guarded vendor suites pass: 24 boundary tests and one auxiliary test, including all three new regressions.
`cargo test --locked -p harmonigraph-plugin --lib --features nice-plug/assert_process_allocs` passes all 158 production plugin tests.
Workspace formatting, semantic Markdown layout and local Markdown links pass.
Final build results are reported in the PR's validation record.
No allocation, race-detector, real-time deadline or macOS/Bitwig claim is inferred from compilation alone.
