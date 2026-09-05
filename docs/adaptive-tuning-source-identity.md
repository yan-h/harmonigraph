# Source identity foundation for #617

This completes only the source-identity subset of [#617](https://github.com/yan-h/harmonigraph/issues/617).
It does not implement tuner aggregation, pairing, the audio-owned session/configuration model, host output acceptance, baseline recovery or adaptive tuning.
The exact committed base is `16e3b1780223d33ebb6a574d2dfc0be33be22cda`, the independently reviewed [stage 2 draft PR #635](https://github.com/yan-h/harmonigraph/pull/635) on `codex/adaptive-tuning-engineering-contracts`.
Full CI and Security audit both passed at that exact base before the first tracked edit.

The implementation branch is `codex/617-source-identity` in the Codex-managed worktree `/Users/yan/.codex/worktrees/d9cf/harmonigraph`.
Its stacked draft PR targets `codex/adaptive-tuning-engineering-contracts`.
The exact pushed head and PR link belong in the PR/final handoff, avoiding a self-referential hash here.
The PR stays draft and is not merged.

## Identity and scope

Core `SourceId(u64)` identifies provenance within one canonical display/take stream.
`SourceId::DIRECT`, value zero, is reserved for the hub's observed direct input;
nonzero identities belong to tuner sources.
The future session owner must allocate a fresh stream identity for each admitted source lease, retain it across that lease's resets/recovery, and never reuse it while old events/history can still exist.
A source slot ordinal is insufficient for this identity.
Allocation, counter exhaustion, runtime session/epoch/incarnation validation and stale-report rejection belong to the session stage before publication.
Recording this stream identity does not persist a runtime authorization token, and replay cannot enroll a recorded source in a live session.

`VoiceKey` is `(source, channel, note)`.
It keys the allocating display tracker's held map, held-end ownership and the roll's independent live map.
Same-source retriggers retain the existing replace-the-held-voice rule and close the old roll entry.
The host note id is still forwarded unchanged by the plugin, including its absence;
it does not become part of the display key.
The source key also reaches scene mark ownership and the deterministic spectral/spiral identity tie-breaks.
The Notes diagnostic pane adds a source column.
All existing direct-input fixtures retain source zero and the same relative ordering.

`SourceReset` releases only the event's source, while `SessionReset` explicitly releases every source.
Both retain existing release fades, held-end stamps and the roll's reset-time clamp.
They ignore channel/key;
session reset also ignores the source field.
The full plugin's existing local reset publishes a direct-input source reset.
It cannot infer a validated session-wide lifecycle boundary for the future session owner.
The standalone's single selected input is also direct input;
switching it stops the old producer, consumes its queued events, then fans out one source reset to the live tracker, log and take before clearing decoder state.
Its existing CC120/123 panic remains source-wide rather than per-channel.

Pitch-only `NoteHistory` and spectral pitch/name caches remain keyed by pitch.
Source does not decide their values, so widening those keys would add churn and split deliberately folded history.
Identity ordering uses the stable source value followed by channel/key;
no frame revision or newly allocated key is introduced.

## One event/control stream and recovery boundary

Source-bearing `NoteEvent` remains the fixed `Copy` payload carried by the display ring and recorder entry.
The event kind now distinguishes ordinary note deltas and scoped reset controls in that same ordered stream.
The editor/background drain copies the entire event and changes only its clock timestamp.
The recorder and standalone share the take crate's core-to-record conversion;
incremental offline replay and `full_roll` share its inverse.
Equal-time source lifecycle order remains the written order under the parser's stable time sort.

Complete recovery is deliberately not represented as a set of ordinary note-ons or as an empty reset followed by fabricated attacks.
The owning session stage must extend this event/control stream with an explicit complete-baseline control at the canonical publication cut specified in [the engineering contract](adaptive-tuning-contracts.md#10-baseline-cut-acknowledgement-and-pending-disposition).
It owes bounded owned baseline payloads, original input/accepted onset and current emitted-pitch metadata, validation of the complete held set, and atomic source replacement in each downstream consumer.
Baseline current-state acceptance must stay distinct from retention/publication of earlier actual events.
This stage adds neither a partial baseline reader nor a second unordered canonical stream, and it claims no runtime baseline handshake.

## Take format break

The take format is now **version 2**.
Every note/control record carries the stream source identity and explicit source/session reset kind.
The parser rejects any header version other than 2, including older version 1 files, with a version error.
This is intentional and has no alias, migration or compatibility shim;
old takes must be recorded again with this build.
Existing saved UI state is unchanged.
The touched persisted header and note-record structs carry container-level defaults, with their fallbacks supplied by `Default`.
Core stays serde-free.

Merely requiring a source field on note records was rejected:
the existing parser can treat an incompatible final record as a truncated write.
An old header must therefore fail before any following record can reach that path.
The focused regression checks old versions with both a complete old final note and a chopped final note.
The normal interrupted-final-line behavior remains available for a supported v2 take.

## Verification and remaining work

The maintained behavior fixtures are:

- `same_key_sources_keep_independent_lifetimes_bends_and_ends`: two sources actually hold channel 0/key 60 at distinct pitches; release, retrigger and source reset on A preserve B's pitch, held-end ownership, settled pitch and bend segments; direct input is separate and a session reset releases all three sources.
- `same_key_sources_mark_their_own_emitted_pitch`: two sources on the same channel/key tune to different represented lattice pitches and claim only their own melody/bass marks.
- `source_scoped_notes_reach_the_take_with_their_original_times`: the actual recorder ring and file writer preserve source, reset scope and fractional timestamps through parsing.
- `source_scopes_and_bends_round_trip_into_incremental_and_full_replay`: serialized same-key sources, release/retrigger, source reset and session reset preserve independent held values and exact roll segments in both replay modes.
- The existing editor clock-drain fixture now uses two sources on the same key and verifies both identities and the original inter-event spacing after clock mapping.
- `an_old_header_is_refused_before_a_final_record_can_look_truncated`: unsupported old takes fail with an audible version error rather than partial success.

Local Cargo workloads are serialized under the sole implementation lease with `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`, using repository sccache and this worktree's target.
The initial workspace all-targets check passed after mechanical fixture updates.
Core/take/scene/record library tests passed:
365 tests, six preexisting ignored probes.
The final handoff records the remaining focused replay/shell checks, formatting and exact-head Actions result.
Full CI runs in GitHub Actions;
no changed golden is blessed without an intended picture change and inspection.

Companion/session/configuration/emitter wiring remains separate work.
No source-incarnation rejection, real-time capacity, callback-cost, host timing or live multi-track acceptance claim is made by this subset.
The original implementer validates and fixes independent review findings against the committed head.
Before pausing, commit first and release-build both `harmonigraph-plugin` and `harmonigraph-offline`, read the actual tag with `./load-plugin.sh --tag`, and leave the shared Bitwig plugin/renderer slots untouched.

## Independent review cycle 1

The coordinator's independent read-only review of `860994fcd0e04923909eaebaf579cf5b6989ba5d` cleared core/display identity, fixture reach and pitch-only cache keys.
The take/replay/standalone reviewer found one P2, confirmed by the original implementer with a failing focused fixture:
frame time 1.000, queued old hardware attack at 1.001, source reset at 1.001, then replacement mock attacks still stamped 1.000. Live state held four mock notes, while the serialized take's time sort moved those attacks before the reset and reconstructed no held notes.

`switch_source` now returns its reset boundary, and the shared input-frame path uses that value for replacement input and the rest of the UI frame.
`source_switch_after_newer_queued_input_replays_the_live_mock_chord` actually queues the newer `RawMidi`, switches to Mock during its sounding phase, records through the standalone writer, parses the file and compares replayed source/key/pitch/onset with the live held chord.
The fixture failed before the repair and passed after it.
No other actionable findings were returned in cycle 1, and no broad Cargo suite was repeated for this fix.
The corrected committed head is returned to the coordinator for bounded cycle 2 review on the same draft PR #636.
