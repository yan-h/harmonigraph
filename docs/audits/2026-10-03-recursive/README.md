# Recursive local audit — 2026-10-03

Completed scoped local audit: source investigation, independent challenge, native tests, focused reproductions and workload measurements.
The ledger distinguishes 19 deeply examined questions, 2 partial performance investigations, 1 blocked native-host question and 1 unexamined other-platform question.
Those limits remain open; this report does not claim universal correctness or stable GPU performance under uncontrolled desktop load.

The strongest structural opportunity is small: publish only CLAP tail length instead of an entire processing-status enum whose atomic cell falls back to a lock.
The current analyzer, scene, recording and renderer ownership generally earns its complexity after recent simplifications.
The evidence does not support another cache framework, recorder rewrite or blanket atlas/resource abstraction.

## Scope and provenance

Audited source: `ad1c6e6b9474dd80098703ae15cac241bba0eb41`.
Dedicated app-owned worktree: `/Users/yan/.codex/worktrees/recursive-audit/harmonigraph`, branch `codex/recursive-audit-2026-10-03`.
Production changes are not part of this audit; temporary assertions are preserved as a guarded patch and have been restored byte-for-byte.
The original checkout, installed plugin, active Bitwig session, user settings and original recordings remain untouched.

Remote main advanced through #1433 and #1431 to `77c453a42`; the exact deltas were reviewed independently.
Roll-inclusive pictures/timings require revalidation on that newer source, and focused keyboard undo changes the host-validation route.
Unrelated pinned owner/lifetime conclusions remain applicable to the inspected deltas.
See [intervening changes](context/intervening-changes.md).

## Ranked shortlist

These are recommendations, not applied fixes; full costs, alternatives and confidence are in [DECISIONS.md](DECISIONS.md).

| Rank | Change | Evidence and practical benefit | Verdict |
| --- | --- | --- | --- |
| 1 | Replace retained full ProcessStatus with atomic tail samples | Only CLAP tail.get reads the field; smaller representation removes a callback-path lock without a new owner. Native query confirms 24 bytes/not lock-free versus lock-free u32; no measured speedup. | Pursue small representation change |
| 2 | Reconcile recording/export and channel-bend documentation | Current guides contradict version 6, arm-time size, camera capture, malformed-look refusal and bend publication. Consolidation removes stale duplicate contracts. | Pursue |
| 3 | Bound Hub discarded-epoch work explicitly | Accepted length does not count discarded pops. Finite probe popped 3072 discarded records versus the 2048 accepted cap; real host stall not established. Entry snapshot gives a local finite bound without persistent state. | Pursue small bound |
| 4 | Make the existing Namer fallback fixture execute Namer | Named test currently calls only the fallback helper. Replace its fixture, rather than grow test machinery. | Optional small improvement |
| 5 | Recover display after malformed NaN audio | Reproduced 2564 poisoned display buckets after two clean windows, with finite/lit raw history; real occurrence unknown. | Small guard only, low priority |

No large rework is recommended.
An optimization earns further implementation only when a representative complete-frame or complete-export effect exceeds the observed noise and its ownership/rebinding cost.

## Validation and measurement limits

The unmodified workspace release suite passed **1910 tests, 0 failures, 27 ignored**, with Metal and FFmpeg required; no golden baselines changed.
The independent adaptive reference model passed **13 tests**.
Release execution does not activate every debug-only allocation guard and is not native Bitwig validation.

Complete encoder-inclusive setup samples took 5.343 seconds at 720p and 8.495 seconds at 1080p for the same 480-frame progression.
A repeated 720p run was interrupted after 138.128 seconds under changed desktop load, before encoder completion.
Those results are not a stable median/spread or evidence of a performance improvement.
The game and DAW were never stopped; the cross-audit lock only serializes our own expensive work.
Later complete repeats ranged from 5.328 to 108.156 seconds at 720p and 7.189 to 97.577 seconds at 1080p; the converted historical workload completed twice in 4.417–4.697 seconds.
Headless UI median frame costs across three runs were 0.675 ms active dock, 0.906 ms dock plus preview and 0.064 ms active without drawing; these exclude native scheduling and GPU preparation.
The halo-only allocation probe measured about 0.253 ms extra CPU preparation, insufficient to justify another resource owner.
See [measurements and raw evidence](MEASUREMENTS.md), including maxima, contexts and every completed/censored sample.

## Coverage and engineering conclusions

[ARCHITECTURE.md](ARCHITECTURE.md) maps workflows and state owners; [COVERAGE.md](COVERAGE.md) provides the index and [coverage.json](coverage.json) records each leaf's scope, investigator, evidence, confidence, uncertainty and next action.
“Deeply examined” applies to the stated question, not universal subsystem correctness.
Native-host work is explicitly blocked by the non-disruption constraint, other platforms are unexamined, and unstable performance measurements remain partial.

- [Audio, analysis reconfiguration and held-note naming](investigations/audio-tuning.md): retain frozen acoustic correction versus current display ownership; bound discarded work.
- [Numerical analysis](investigations/numerical-analysis.md): finite-input calibration, silence, interpolation and history energy checks support the current model; retain accepted Fast/Fold limits.
- [Realtime payload lifetime](investigations/realtime-lifetime.md): fixed-value adoption and off-audio heap lifetime have distinct jobs; allocation-fixture reach is explicitly qualified.
- [UI, scene and hidden lifecycle](investigations/state-scene.md): pending restore order, normalized Scene.view and per-surface histories have concrete reasons to remain.
- [Rendering and atlas lifetime](investigations/render.md), [shadows/glows](investigations/shadows-glows.md): current asymmetries survived source and fixture challenge; target splitting lacks a demonstrated net benefit.
- [Recording/replay](investigations/recording-replay.md), [temporal cadence](investigations/temporal-cadence.md): retain distinct closure frontiers and shared runtime; event equivalence is not arbitrary pixel equivalence, and trimmed exports intentionally start audio history cold.
- [Development workflow](investigations/development.md): recent fixes and known tag/Linux issues are acknowledged; no measured case for changing worktree/cache/CI architecture.
- [Independent challenge](investigations/challenge.md): records rejected stronger claims, competing Hub bounds and tail-publication edge cases.

## Continuation ordered by expected value

1. Implement the atomic-tail simplification with the exported-tail/status fixture specified by independent challenge.
2. Correct and consolidate the current recording/export and channel-bend documentation.
3. Add the Hub entry bound with bounded-replenishment/queued-tail and mixed-peer progress coverage; no scheduler or handshake.
4. Fold the optional Namer fixture strengthening into relevant test work; address NaN recovery only with the small local guard described in the follow-up.
5. Only if a user-visible hitch is observed, capture its exact look/take and measure complete frames or encoder-inclusive exports on current main under usable load. Revisit target retention/readback only with that evidence.
6. Run selected [native-host procedures](HOST-VALIDATION.md) in a disposable project when the owner is ready; the audit does not require disrupting the current session.

[REPRODUCE.md](REPRODUCE.md) contains exact commands and workload transformations.
The scripts, test-only patch, logs and context snapshots are preserved; source recordings and rendered media remain ignored local artifacts.
Existing #886, #1348, #1425, #1428/#1430 are linked in the investigations rather than duplicated.

## Follow-up tracking

The evidence is preserved in [draft PR #1434](https://github.com/yan-h/harmonigraph/pull/1434); it is open, draft and not merged at handoff.
[Tracking issue #1439](https://github.com/yan-h/harmonigraph/issues/1439) indexes every area and its remaining limits.
No implementation recommendation has been recorded as an accepted owner decision.

- [#1435: publish only CLAP tail length](https://github.com/yan-h/harmonigraph/issues/1435).
- [#1438: reconcile recording/export and bend documentation](https://github.com/yan-h/harmonigraph/issues/1438).
- [#1436: bound discarded-epoch Hub work](https://github.com/yan-h/harmonigraph/issues/1436).
- [#1437: recover spectral display after NaN input](https://github.com/yan-h/harmonigraph/issues/1437).

The optional Namer fixture improvement remains in the tracking issue rather than a separate runtime-defect issue.
