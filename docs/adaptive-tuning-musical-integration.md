# Production musical integration

This #621 stage is stacked on ordinary recovery commit `4623067b99c81249ca27be7cf8b93ffb953c4db4`.
It replaces the artificial central correction with the reviewed [version-one musical policy](adaptive-tuning-policy-engine.md).
The [user's ordinary-use priorities](adaptive-tuning.md#design-priorities) continue to govern implementation and review.
This stage does not establish complete host qualification or finish the combined adaptive-tuning objective.

## Trying the current build

1. Load the matching plugin/renderer pair and confirm its build tag using the handoff.
Use the documented Bitwig **by Vendor** process topology, with individual hosting overrides off for both Harmonigraph classes, at 44,100 Hz and a fixed 512-sample engine buffer.
The [measured topology and calibration restrictions](adaptive-tuning.md#session-pairing-and-process-boundary) apply.
2. Place one full Harmonigraph on Master and one Harmonigraph Tune in the note-effect path before each instrument.
Keep **Participating** on in each Tune editor.
**Automatic: exactly one hub** pairs a unique compatible hub;
choose the intended hub explicitly when needed and resolve missing/ambiguous status before playing.
3. Hub and Tune start automatically with signed offset zero and read sample rate and maximum buffer size from host activation.
No validation checkbox or first-use Apply is required.
The Hub's **Session** menu and each Tune editor expose an advanced signed sample offset for a known routing delay;
use **Apply / Reinitialize** after changing that offset.
Automatic setup does not measure graph or delay-compensation latency.
Actual host format, steady-time continuity and arithmetic checks still suspend playback when their clock evidence fails.
After a host rate or buffer change, reload the plugin instances to start a fresh session with the new automatic format.
Live reactivation still preserves old ownership and may remain pending at the existing settlement boundary, tracked in [issue #703](https://github.com/yan-h/harmonigraph/issues/703).
Old saved rate, buffer and manual validation keys are ignored and are no longer written;
saved routing retains only its signed offset.
4. In the Hub's **Tuning** pane, turn both **Auto** switches and **Learn** off, then press **Just** to release both temperament locks and set the independent Just axes.
Use a zero C offset for comparison with the fixtures.
The fresh locked 12-TET defaults intentionally yield zero adaptive correction, so they cannot demonstrate Just retuning.
5. Start with a small phrase on three tracks, including simultaneous D/F/A, and check the instrument's actual pitch as well as the displayed notes.
The known-neutral callback fixture selects D/F/A corrections of +3.910034, +19.551330 and +5.865051 cents with independent Just axes.
Destination conversion matters:
the [historical pitch-conversion measurements](tuning-probe-bitwig.md#pitch-conversion) include Vital's required matching MPE settings and bend ranges.
Check Stop, Off, project reopening, and live/offline output in the actual project before treating the new musical build as host-qualified.

Fresh Bitwig controller initialization is still unverified.
The ordinary unknown-pedal second-phrase limitation in [#696](https://github.com/yan-h/harmonigraph/issues/696) remains;
known-neutral fixtures and the settings above do not prove that the host initializes CC64/66/69.

The first-use callback regression starts three Tunes before the Hub, then resets the Hub before its first audio callback, matching the silent live instance's startup ordering.
Registry offers initially carry a provisional epoch;
a Tune now waits for the Hub's first published audio progress before sending its first adoption, baseline or input prefix and uses that established epoch.
A Hub becoming ready during the Tune callback cannot turn this wait into a clock fault.
Already published/adopted streams retain their existing epoch recovery rules.
The regression runs both fresh defaults and real restores containing obsolete `validated: false` and zero/mismatched format keys, accepting fifteen Just-tuned notes and their releases without a setup click.
Its first C/E/G On/Off gesture and actual neutral controller inputs arrive after Hub registration/reset but before its first audio callback;
after enrollment, the original notes emerge with their release spacing preserved.
These earliest inputs can miss D512 and enter the existing sliced lateness recovery;
the fixture proves that recovery settles before its four later gestures.
This fixes the measured initial enrollment failure;
it does not claim every setup wait or live reactivation path is resolved.

Configuration-consumer tests explicitly select the existing observation-only apparatus, preserving their intentional marker-retention and publication-loss scenarios.
The older D0 aggregation apparatus supplies a simulated initialized Hub clock;
it does not test first enrollment.
Factory-default startup, ordinary musical output and DIRECT zero-delay/reactivation fixtures do not use those overrides.
A separate raw-forwarding fixture includes 1,025 events at one sample, above the musical cohort limit of 1,024.
With production sequencing that shape fills DIRECT ingress before its complete-sample proof can advance;
physical forwarding completes but capture retirement stalls, recorded in [issue #707](https://github.com/yan-h/harmonigraph/issues/707).
The raw-forwarding test therefore explicitly isolates observation behavior;
no capacity increase or partial-cohort rule is included here.

## Production diagnostics

The production plugin emits `HG-TUNING` summaries through the existing Info-level plugin logger, including when editors are closed.
Its default sink is stderr, which Bitwig includes in its engine log (`~/Library/Logs/Bitwig/engine.log` in the measured host); an existing `NICE_LOG` override still applies.
`HG-TUNING-SETUP` records an accepted Apply/Reinitialize or restore immediately on the main thread, with the prior adopted clock.
Summaries include PID, registration, build tag, selected Hub and runtime Source/session identities so three Tune instances remain distinguishable.
The actual live Bitwig summary capture is a handoff check; the exported wrapper fixture has verified summary and blocked-Apply output through the same stderr logger.

Audio callbacks update fixed counters and publish fixed atomic snapshots at approximately one-second audio intervals.
The existing setup callback performs all formatting and logging on the main thread, at most once per wall-clock second when the report's meaningful values change.
Absolute clocks do not cause idle logging;
coverage lag, rounded to seconds for the change key, still exposes a stalled peer.
No editor, worker, new transport message or persisted field is involved, and diagnostics never mark setup dirty.

Each Source reports consumed note On/Off counts, accepted note output counts and last actual pitch, received assignments/correction, adoption, retained work, and the actual setup wait.
`last_output_player_mc` and `last_output_correction_mc` come from the same factual voice as `last_output_key` and `last_output_pitch_mc`;
`last_correction_mc` instead describes the most recently received assignment and can belong to another note.
Each Hub reports the last published Source/key/pitch together, resolved tuning axes/locks/Auto/Learn, transition/input/publication wait reasons and source slot, credits, and each row's membership, coverage and retention.
`*_mc` values are microcents (1,000,000 per cent); sample frontiers use the shared mapped clock, `raw_end` uses the host clock, and the minimum signed integer denotes an absent frontier.
Counters are cumulative per instance; a last note remains historical after its release.
An `Adopted` setup clock and the registry's Attached status do not by themselves prove that a row has completed audio enrollment.

Named setup/transition/input/publication reasons are printed beside their numeric codes.
The remaining compact masks use these bit positions:

| Field | Bits, from bit 0 upward |
| --- | --- |
| `pending_gate` | Invalid clock; admission closed; assignment unavailable; cancellation cut; prefix reconciliation; prefix ordering; emergency fault; locally completed envelope retained for remote history |
| `output_settlement_wait` | Held credit; held pedals; owed Note-Off; ordinary journal; emergency journal; prepared permit; manifest; emergency slots; channel reset; baseline |
| Source `recovery` | Fence pending; inventory counting; revoke acknowledgment owed; inventory acknowledgment owed; cleanup pending |

A nonzero settlement mask during an ordinary held note is expected.
The setup wait and accepted/adopted generations establish whether that retained state is currently preventing Apply.
The focused regression uses three exported Tune instances and a Hub, proves the periodic host callback after initial setup is drained, verifies distinct identities and nonzero Just tuning, then pauses Hub consumption after an actual Apply and observes the retained-history wait.
It also checks that a later pitch expression updates the Hub's published identity and pitch together, and that healthy idle clock progress leaves the log change key unchanged.

## Owned musical state

`ConfigReducer` binds `policy::CONFIG` version 1 with the complete resolved configuration.
The existing sequencer calls `assign_new_note` once for each selected adaptive onset;
unaccepted suffixes can be evaluated again during existing late-delivery recovery.
Each call sees all eligible factual voices and scheduled predecessors, using their current exact pitch including player expression.
Attack nodes are optional provenance, never substitutes for that pitch.
The chosen correction, node or completed NoCandidate, configuration, initial player expression and decision travel in the existing addressed Assignment/Plan/Life path.
Held assignments retain their configuration and correction through later configuration edits and bends.

The Hub owns separate confirmed and prospective history tables indexed by Source slot, channel and key, with incarnation, musical revision and decision identity.
Prospective history updates after successful selection preflight and before the next onset.
Confirmed history updates only through the existing exact accepted-output binding check.
Release preserves key history.
Configuration revision, source withdrawal/reset and recovery/clock boundaries advance decision floors;
old accepted output cannot repopulate a cleared table.
Display-tolerance changes preserve the musical revision and history.
NoCandidate is a completed musical clear, while intentionally unretuned Off births do not update musical history.

Participation changes reserve an Original in the existing Capture arena before changing the local flag or acknowledging setup.
That source-wide marker orders Off relative to its own earlier and later events.
The Hub removes the source from future scoring when the marker is sequenced, while retaining its factual voices, output records, credits and release obligations.
An onset's immutable birth mode still decides whether a pre-Off request receives its adaptive assignment.
Later factual baselines cannot reverse an already sequenced participation marker.

Normal canonical deltas now carry optional assignment revision, decision, node, frozen correction and current player tuning.
The Hub applies accepted binding metadata before normal display/take publication;
it does not substitute an intended correction for rejected initial tuning.
Baselines carry the same decision alongside their existing configuration and node.
The tracker and `.take` reader preserve these fields.
DIRECT has no musical assignment and retains a compact observation in its existing output queue.

The take fields are additive under existing container defaults, and the format remains version 4.
Earlier version-4 records have no new decision provenance;
zero decision means no established musical metadata, and no node or decision is invented.
An older reader can still read the accepted pitches but does not retain the new optional metadata.
No saved UI shape, compatibility alias or migration is added.
Fresh resolved configurations now identify policy version 1 instead of the artificial version 0.

## Executed functional evidence

The six exported-CLAP musical fixtures use the real Hub and Tune classes, configuration commands without opening the editor, normal accepted output and callback allocation/deallocation guards.
They explicitly initialize accepted neutral CC64/66/69 on their used channels;
this is not evidence about fresh Bitwig initialization.

- All six source orders for same-sample D/F/A, with reversed reply drains, select `(2,0,0)`, `(3,-1,0)` and `(3,0,0)`.
Each successor includes the previous assignment, and accepted Source/Hub pitch and provenance agree.
- An established E at 5/4 survives release and a tolerance edit when repeated against D.
The corresponding fresh Source or channel selects `(4,0,0)`, and Off clears the original preference.
- Supported independent axes 720/360/960 cents produce NoCandidate for MIDI 63.
Its nonzero decision, zero adaptive correction and +0.375-semitone player expression survive actual output.
- Off retains a sounding D's correction, bend and physical release while excluding it from a simultaneous foreign E's score.
- A normal 15-note phrase across three Sources retains a bent C and old configurations.
After changing the fifth axis to 690 cents, the next F-sharp selects `(2,1,0)` with correction −33,686,279 microcents.
The explicit counterfactual using stale attack pitches selects `(-2,-1,0)` and +33,686,279, so this fixture distinguishes actual-pitch context from node-derived context.
The normal display stream, actual written take and read/replayed take agree with Source facts for all 16 voices, without a voiced repair baseline supplying the metadata.
- A real musical configuration revision and a Source Stop that releases a held D and enters recovery each clear released-key history while retaining correct factual output.
The configuration case leaves already held assignments unchanged.
An isolated discriminator with all voices released before Stop retains the E preference, stays in epoch 1 and has no loss recovery or output fault.
That is consistent with the specified withdrawal/configuration/session/epoch/loss-recovery resets;
the fixture does not claim that every silent Stop erases history.

Focused tests also verify every canonical coordinate under all four respelling forms fits the exact compact representation, and that confirmed history is separate from prospective history and cannot undo a clear.
The existing recovery, partial-output, deadline, native Off and capture-retirement fixtures retain their ownership assertions.
The partial-onset fixture rejects a real Just B correction of −0.11731262 semitones and verifies that the accepted raw B and take contain no intended assignment metadata.
Old artificial correction expectations now use real musical values;
capture serial expectations include the newly retained participation marker.

## Storage

The isolated production allocator fixture runs with process allocation guards, two build jobs and two test threads, and measures requested bytes on the calling thread.
Allocator-private overhead and background/native allocations are not a session heap measurement.
The measured ordinary subtotal is **109,054,344 bytes**, or **436,217,376 for four sessions**, compared with 106,201,496 at the base.

| Physical increase over the base | Bytes |
| --- | ---: |
| Separate confirmed/prospective history, two × 34,816 × 40 | 2,785,280 |
| PolicyScratch plus 256 ContextPitch inputs | 9,328 |
| Two voice arrays, 256 × (72 − 40) each | 16,384 |
| Sequencer inline owner, 6,224 − 5,840 | 384 |
| Seventeen Source State bodies, 64 × 8 each | 8,704 |
| Sixteen Row State/cached-baseline bodies, 2 × 64 × 8 each | 16,384 |
| Sixteen SourceControl pairs of baselines, 2 × 64 × 8 each | 16,384 |
| Total ordinary increase | 2,852,848 |

The four new backing allocations are the two histories, policy scratch and input context.
They consume their existing reservations, not new pools outside the ledger.
The richer voice arrays replace the node/decision placeholders and eight bytes of reserved configuration with the actual revision:
confirmed ownership remains `72 + 64 + (128 − 8) = 256` bytes per voice across the factual cache and ConfirmedPitches.
The distinct prospective half remains reserved in full.

| Executed layout | Bytes | Unchanged ceiling |
| --- | ---: | ---: |
| Allocated Plan / Life cell | 192 / 192 | 256 including future 128-byte configuration |
| History cell | 40 | 64 |
| Publication item | 224 | 256 |
| Whole publication baseline slot | 15,816 | 16,384 |
| Whole protocol/cached baseline | 15,824 | 16,384 |
| SourceControl | 32,272 | Two prepaid complete baseline slots plus existing controls |
| Inventory slot + Arc + full config expansion + manifest | 12,944 | 16,384 |
| Wrapper output group | 248 | 256 |
| Observed DIRECT output cell | 120 | 128 |

The larger Source State, receiver State and cached/owned baseline copies fit their independently reserved voice/baseline rows in the [full allocation account](adaptive-tuning-aggregation-memory.md#constrained-future-allocation-plan).
The new 384-byte sequencer metadata and two 48-byte publication-consumer increases consume 480 bytes of the existing 65,536-byte fixed-metadata reservation once.
The capped allocation plan remains **150,730,867 bytes**, leaving **264,077 below 144 MiB**, conditional on the existing joint record and enclosing-frame layout constraints.
The 128-byte configuration ceiling is not required padding and cannot be spent independently inside every enclosing baseline.
With the present representation, expanding each embedded configuration from its complete current 64 bytes to 128 would make VoiceBaseline 272 bytes and the publication baseline slot 19,912 bytes, exceeding both 256 and 16,384.
The same future-only limitation already existed at the base;
this change does not claim that the unchanged baseline representation supports that expansion.
All Plan/Life configuration allowances, both history/voice halves, future Plan indices and the complete 4 MiB policy reservation remain charged.
Future shape changes must satisfy both their nested-record ceilings and complete-frame ceilings together, as the original allocation contract requires.
Neither smaller DIRECT storage nor earlier inventory/GUI savings are reclaimed again.

Outside the ordinary subtotal, both actual publication channels now request 2,911,744 bytes, up 428,032 within their prepaid ring/baseline banks.
The boxed configuration owner plus DIRECT queue requests 498,264 bytes, down 15,872 because its compact output cells save 16,384 while its inline State grows 512.
Actual full-HG factory retention is 63,797,509 bytes plus 760 activation bytes;
sixteen Tune factories with activation retain 70,913,963.
The constructed/enrolled four-HG plus 64-Tune population retains 538,058,238 bytes on the measured thread.
These include existing plugin/UI/worker integration and fixture overhead;
they are not interchangeable with the additional adaptive-session ceiling.
Factory probes begin with empty non-RT trackers;
additional populated display/replay voice storage is outside those measured empty-factory totals.

## Timing and limits

These are single-run observations on the local Apple Silicon machine, with Rust 1.92, two build jobs, two test threads and exclusive local Cargo work.
Debug executions enable `nice-plug/assert_process_allocs` and exercise the callback allocation/deallocation guard.
The feature is also specified for release runs, but the vendor guard is gated by `debug_assertions`;
release runs measure callback cost and do not constitute a second allocation-guard receipt.

| Release workload | Maximum Source callback | Maximum Hub callback | Sum of measured callbacks through delivery |
| --- | ---: | ---: | ---: |
| Ordinary 15 notes / three Sources, bend, config edit and successor | 0.015417 ms | 0.108000 ms | Not accumulated |
| Supported independent Just, 16 Sources / 256 onsets | 0.222125 ms | 2.150750 ms | 22.293378 ms |
| Synthetic 65-candidate arithmetic, 16 Sources / 256 onsets | 0.139583 ms | 2.274875 ms | 24.603637 ms |

The maxima are observed callback time, not a worst-case execution guarantee or Bitwig measurement.
The sum spans many callbacks and is not one callback's cost.
The ordinary phrase reaches the real source/Hub path with 15 held voices before its successor, the accepted seven-semitone bend, old configurations and normal display/take publication.
Its three Source maxima were 13,625, 7,000 and 15,417 ns.

Debug guarded and release maximum-capacity executions prove that 256 accepted onsets across 16 Sources can settle exact credit and cleanup.
The supported independent-Just configuration has 5–7 candidates for the used key classes, and the final onset reaches 255 context voices.
The existing late-delivery path evaluates 288 policy calls with 40,304 aggregate context inputs because 32 unaccepted requests are evaluated again.
It does not deliver this maximum-capacity workload at D512.
Input is sample 1,536 and due is 2,048.
In both release capacity cases, Sources 0–13 accept at sample 16,384 and Sources 14–15 at 58,880.
Every accepted onset offset is zero within its callback, so the extra delay is exactly 14,336 or 56,832 samples, about 325 ms or 1.289 s at 44.1 kHz.
These are output scheduling delays, separate from CPU time and from the ordinary workload.

An explicitly synthetic all-zero-axis configuration, injected through the existing test-only serialized owner seam before any note, reaches all 65 candidates on the same production callback path.
It is outside the supported UI range and is an arithmetic stress measurement, not a user setting or a host throughput claim.
Both cases retain the exact 256-output, no-duplicate, held-credit and final zero-credit assertions.

The local receipts use the following filters with `RUSTC_WRAPPER='' CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2` after the configured sccache was denied before compilation:

```text
cargo test -p harmonigraph-plugin --features nice-plug/assert_process_allocs --lib
cargo test -p harmonigraph-plugin --features nice-plug/assert_process_allocs production_musical -- --nocapture
cargo test -p harmonigraph-plugin --features nice-plug/assert_process_allocs measured_ordinary_storage_and_actual_factory_allocation_increments -- --nocapture
cargo test -p harmonigraph-core -p harmonigraph-record -p harmonigraph-take
cargo clippy --workspace --all-targets -- -D warnings
cargo test --release -p harmonigraph-plugin --features nice-plug/assert_process_allocs production_musical -- --nocapture
cargo test --release -p harmonigraph-plugin --features nice-plug/assert_process_allocs production_sixteen_sources_complete_a_256_onset_cohort_and_hold_exact_credit -- --nocapture
```

The synthetic run adds `HARMONIGRAPH_MUSICAL_MAX=1` to the last command.
The full guarded plugin run passed 203 tests and exposed one stale hard-coded capture serial;
its corrected isolated fixture passed with the original ownership/reuse assertions intact.
Core passed 137 unit and two integration tests with one existing ignored test, record passed 59, and take passed 17.
Workspace all-target clippy passed with warnings denied.
The six musical release fixtures and both capacity runs passed.
The PR handoff carries exact committed-head CI and both-package release receipts.

The prior ordinary recovery fenced successor took 20,592 extra samples, about 467 ms, beyond its due time.
Healthy D512 phrase tests do not erase that measured recovery limitation.
[Issue #696](https://github.com/yan-h/harmonigraph/issues/696)'s unknown-pedal second-phrase problem remains an ordinary note-only limitation deferred for the complexity of changing the accepted-neutral contract.
Host rate and buffer values are now automatic, removing the stale zero/mismatched-format variant of [issue #692](https://github.com/yan-h/harmonigraph/issues/692).
The broader invalid-routing Reset sequence remains deferred;
this change does not add setup supersession or infer routing-offset validation.
No new controller initialization, reconciliation framework or extreme stress matrix is added here.

Fresh Bitwig controller initialization, listening, practical live/offline destination behavior and complete combined independent review remain outstanding.
Both plugin and offline release artifacts are required for the handoff;
this stage never swaps the shared DAW or renderer slot and never merges its draft PR.
