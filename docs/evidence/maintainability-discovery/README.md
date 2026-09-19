# Maintainability discovery probes

These are unapplied experiment patches and one standalone discovery probe,
not production changes or an additional test suite.
They preserve the costly reproduction work behind [#954](https://github.com/yan-h/harmonigraph/issues/954),
[#955](https://github.com/yan-h/harmonigraph/issues/955) and [#957](https://github.com/yan-h/harmonigraph/issues/957).
Evidence was prepared in [PR #951](https://github.com/yan-h/harmonigraph/pull/951);
product implementation is separate.

The subsequent [historical inventory](history-inventory.md) records file/line scope for 100 merged changes.
It is read-only history evidence,
not another runtime probe;
its [interpretation and recommendations](../../historical-change-cost.md) separate generated output,
moves and justified multi-file changes from recurring maintenance obligations.

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

## Continued constraints and dependency inventory

The next pass inspected `fcea7dd11e133de50ccd02e2999ad79f5fe5f30f` on 2026-09-19,
without changing product source.
It reused two existing regressions to challenge current prose in `docs/visual-runtime.md`:

```sh
RUSTC_WRAPPER='' cargo test --offline -p harmonigraph-ui a_fold_outlives_a_frame_that_brought_no_new_column -- --nocapture
RUSTC_WRAPPER='' cargo test --offline -p harmonigraph-plugin a_take_ends_itself_while_the_editor_window_is_shut -- --nocapture
```

**Observed:** one test passed in each command.
The first proves ten later frames reuse the Fold measurement until new audio arrives;
the second reaches the closed-window completion poll.
[#966](https://github.com/yan-h/harmonigraph/issues/966) holds the stale-guide corrections.
No new test or source mutation was needed.

The dependency check used the [official registry entry](https://index.crates.io/eg/ui/egui-baseview) and [published 0.7.2 archive](https://static.crates.io/crates/egui-baseview/egui-baseview-0.7.2.crate).
The registry marks 0.7.2 non-yanked and requires egui/egui-wgpu `^0.36.1`,
wgpu `^30` and baseview `^0.3.4`.
Downloaded archive SHA-256 equals its registry checksum:

```text
f197eade3efd9bfe76b9cbf8357beb6a48e0a2ac262901b0f52c736e2710fdaf
```

The crates.io API returned 403;
the sparse index and verified archive supplied the release evidence.
Moving upstream main was separately observed at `877d3bba377ce194d1dda8599377ed667bc2d6d7`,
but comparisons use the published archive.
Source was extracted for inspection only,
not compiled or installed.

[#968](https://github.com/yan-h/harmonigraph/issues/968) records the bounded comparison:
physical/logical resize looks structurally replaced,
texture-delta handling needs a runtime probe,
and occluded upload submission,
MSAA reconfiguration and presentation retry remain adaptation questions.
The old “reconsider when egui 0.36 is published” trigger is met;
the cost and value of the whole migration remain unmeasured.
No upstream issue or PR was submitted.

The support audit found the narrow macOS/Bitwig scope already explicit.
Its portable-code and generic-feature-name hypotheses did not establish a new support obligation.
[#967](https://github.com/yan-h/harmonigraph/issues/967) separately records the concrete conflict between old audit dispatch instructions and current worktree ownership.
The full issue index and retained/rejected hypotheses remain in [#954](https://github.com/yan-h/harmonigraph/issues/954).

## Eligible neighborhood versus exact winners

The production reachability API was subsequently retired in #970.
Run this historical probe against the audited pre-retirement revision named below,
not current sources.

This continuation was run on 2026-09-19 at `2a5595fd`,
with product sources still at `1786c5ba`.
[reachability-probe.rs](reachability-probe.rs) compares current production candidate preparation with the exact reachable-winner solver.
It supplies the context immediately before the final onset in the existing `minor` and `fifths-high` examples in `crates/harmonigraph-core/src/policy/fixtures.txt`.
The snapshots use the fixture oracle's integer pitches and coordinates,
not a live Hub capture or a new sequential-policy run.
All context weights are one;
the fifth-chain reference is its last onset's output-minus-input.
Configuration uses the default policy with just tuning and no tempered commas.

Run from the repository root at that revision:

```sh
probe_dir=$(mktemp -d)
rustc --crate-name harmonigraph_core --crate-type rlib --edition=2021 -O crates/harmonigraph-core/src/lib.rs --out-dir "$probe_dir"
rustc --edition=2021 -O docs/evidence/maintainability-discovery/reachability-probe.rs --extern harmonigraph_core="$probe_dir/libharmonigraph_core.rlib" -o "$probe_dir/probe"
"$probe_dir/probe"
```

This builds only the dependency-free core and scratch executable;
no tracked product files or Cargo manifests/locks are modified.
The probe asserts every winner belongs to the eligible set and prints set sizes plus example extra coordinates.
Observed with the production solver over 3600–9600 cents (C2–C7):

| Snapshot | Eligible | Winners | Extra eligible outlines |
| --- | --- | --- | --- |
| minor-before-bb | 36 | 23 | 13 |
| fifths-high-before-e | 46 | 24 | 22 |

For example,
`(-3, 0, 0)` and `(-2, -1, 0)` belong to the eligible set but not the winner set in both snapshots.
Coordinates identify lattice spellings;
letter names alone can hide comma differences.
These counts include nodes outside the current camera,
so they are not screen counts.
No production UI change was made.

The existing JavaScript simulator independently returned the same counts on these two examples,
but it lacks the production keyboard filter and therefore is not evidence of general plugin parity.
The compiled production probe is the evidence used in the [requirements audit](../../requirements-value-audit.md).
It establishes a visible semantic tradeoff at two named contexts,
not a CPU improvement, worker deletion or complete reachability validation.
