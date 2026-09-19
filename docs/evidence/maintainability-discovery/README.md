# Maintainability discovery probes

These are unapplied experiment patches,
not production changes or an additional test suite.
They preserve the costly reproduction work behind [#954](https://github.com/yan-h/harmonigraph/issues/954),
[#955](https://github.com/yan-h/harmonigraph/issues/955) and [#957](https://github.com/yan-h/harmonigraph/issues/957).
Implementation remains deferred in [draft PR #951](https://github.com/yan-h/harmonigraph/pull/951).

Measured on 2026-09-19,
macOS arm64 with Rust 1.92,
at `38ad34eedbfe40d164f77a0012e33c656f3de4a8`.
That documentation branch contains product revision `1786c5ba1a4e2994c20dc2672eb7d200fd605d9b`,
including #946 and #953's note-history repairs.
All temporary source,
manifest and lockfile edits were restored after each experiment.

Use an isolated owner-managed worktree at that revision for reproduction.
Apply each independent experiment to clean source with `git apply`;
reverse its patches before trying another,
and restore any Cargo.lock update caused by the temporary dev dependency.
These patches deliberately include failing controls and unfinished prototypes.
Do not commit their applied state as an implementation.
Commands use `RUSTC_WRAPPER=''` because the local sandbox could not start sccache.

## Dormant export fields

Apply [dormant-export.patch](dormant-export.patch).
Run:

```sh
RUSTC_WRAPPER='' cargo test --offline -p harmonigraph-record discovery_dormant_export_controls_are_inert_with_active_control -- --nocapture
```

**Observed:** one test passed.
All 16 combinations of the four dormant fields produced the same complete `RenderRequest` values through both automatic and manual constructors.
The positive control doubled resolution and produced a different request.
This verifies request construction;
it does not launch ffmpeg or test every arbitrary string value.
Source inspection separately establishes that the production capture path does not read these fields.

## CLAP tracing and fixture initialization

Baseline command:

```sh
RUSTC_WRAPPER='' cargo test --offline --manifest-path vendor/nice-plug/Cargo.toml --target-dir target/debug/vendor-nice-plug --features assert_process_allocs,clap-boundary-tests --test clap_boundary
```

**Observed:** the unmodified suite passed all 21 tests.
A scratch witness with tracing still enabled also passed all 21.
Apply [trace-off.patch](trace-off.patch),
which disables the hook and asserts that no trace callbacks happened.
The suite aborts with `memory allocation of 64 bytes failed`.
Adding `-- --test-threads=1 --nocapture` isolates `deferred_gui_producer_finishing_after_audio_still_wakes_host`.

The allocator already enables backtrace and log support.
A temporary stderr logger exposed allocation in `std::sync::Mutex::lock` from the fixture's `clap_performance_begin`,
initializing `Control.observed`.
The trace hook had incidentally initialized that mutex before the process guard.
The stack and eliminated hypotheses are recorded in [#955](https://github.com/yan-h/harmonigraph/issues/955).

On top of the trace-off patch,
apply [trace-fixture-initialization.patch](trace-fixture-initialization.patch).
It explicitly initializes only that fixture mutex in `Device::new`.
**Observed:** all 21 tests passed with tracing off and allocation assertions still enabled.
This covers the existing behavioral assertions without the tracing path;
it is not a completed API deletion or a cross-platform measurement.

## Nested settings coverage

Use this command with the filters below:

```sh
RUSTC_WRAPPER='' cargo test --offline -p harmonigraph-ui FILTER -- --nocapture
```

The baseline filter `the_loaded_state_guard_poisons_every_dialled_view_float` passed.
Each row starts from clean source unless two patches are listed together.

| Experiment | Filter | Observed result |
| --- | --- | --- |
| [Omit top-level render scale](omit-top-level.patch) | `the_loaded_state_guard_poisons_every_dialled_view_float` | Failed naming `render_scale`; the existing check is sensitive to top-level omissions. |
| [Omit nested cloud depth](omit-nested-atmosphere.patch) | Same guard, then `loaded_settings_low_fit_real_bars`, `loaded_settings_high_fit_real_bars`, `fresh_settings_fit_real_bars` separately | All four passed; the valid default masks the omitted hostile input. |
| [Extend the existing shape comparison](nested-inclusion-prototype.patch) | `tests::settings_ranges::` | All five tests passed, preserving the conditional UI matrix. |
| Extension plus [omit cloud depth](prototype-omit-cloud.patch) | `the_loaded_state_guard_poisons_every_dialled_view_float` | Failed naming `spectrum.atmosphere ["cloud_depth"]`. |

The prototype checks existing nested owners using the same serialized-field method.
It demonstrates feasibility,
not a finished design:
it leaves an unused import,
uses a provisional aggregate count floor,
and still depends on RON's float spelling.
It does not automatically classify mixed structs or discover future nested owners.
[#957](https://github.com/yan-h/harmonigraph/issues/957) records the bounded follow-up.

## Recorder to live display and file replay

Apply [parity-probe.patch](parity-probe.patch),
which reuses the recorder's existing test-support API through a temporary dev dependency.
Run:

```sh
RUSTC_WRAPPER='' cargo test --offline -p harmonigraph-offline recorder_display_and_disk_replay -- --nocapture
```

**Observed:** both tests passed.
The ordinary stream includes notes,
a bend,
a release and source reset.
The canonical stream includes exact pitch,
routing,
a publication gap,
held and empty baselines,
and completion.
Both compare real display tracking with the real writer pump → actual file → `Take::read` → offline `Replay`,
normalizing a ten-second clock offset and checking independent expected note values.

Two separately restored negative controls were executed:

- In the ordinary probe, replace `live.time - ORIGIN` with `live.time - ORIGIN + 0.125` at the `recorder.note` call. The ordinary comparison fails with shifted note times; the canonical test still passes.
- In `crates/harmonigraph-take/src/canonical.rs`, change only `impl From<DeltaRecord> for NoteDelta` to set `pitch_microcents: None`. The canonical comparison fails: the disk path has a 159-semitone segment where the live path has 60.375. Its final pitch still agrees after baseline repair, so comparing the trajectory is material. The ordinary test still passes.

The initial ordinary fixture failed to finalize because it omitted `finish_callback` after `is_armed`;
the preserved patch pairs those calls as the current API requires.
That was a fixture setup error,
not a product defect.
The test-support pump uses the writer's actual implementation but does not start its command thread.
No DAW,
host-input adapter,
Hub producer,
rendered frames or audio samples are exercised.
The compared fields describe the note roll and gap presence at one final time,
not complete equivalence of every runtime subsystem or cadence.

## What is intentionally absent

No permanent test runner,
new metadata system,
production API or compatibility shim was added.
The patches are evidence for a later selected change;
they are not maintained as a second green test suite.
The full allocation backtrace and the relevant failing outputs are summarized in the linked issues,
and the discovery report states the limits of every conclusion.
