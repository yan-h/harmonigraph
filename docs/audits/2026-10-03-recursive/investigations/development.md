# D1 — build and development workflow

Read Cargo.toml, .cargo/config.toml, ci.sh, .agent-lifecycle.json, session-lifecycle.sh, prior #1201/#1407 and docs/gui-cohort-feasibility.md.
No new material workflow rework established.

Recent paid costs already removed: leaf build-tag ownership (#1209), offscreen ink allocation lifetime (#1389), shared shader/pipeline construction (#1213/#1224), redundant CI render tests (#922), and release cross-crate LTO whose compile cost had no measured export benefit.
The current CI split intentionally caps two macOS jobs per PR under a shared account runner limit; adding jobs would trade one PR's latency for queue pressure across concurrent PRs.
Current sccache setup supports independent worktree targets; this run uses a single release test build with three compiler jobs, reused by all investigators.
Cold/feature-mismatched compilation here is an audit setup cost, not evidence to merge target directories or disable caching.
No timings compare alternative build architectures.

The installed GUI cohort is deliberately patched for native input/resize/repaint/lifecycle contracts.
A historical upstream feasibility prototype found migration work and lacked successful visible presentation; it does not authorize dropping those patches today.
No dependency version recommendation is made without a fresh upstream/native qualification.

Existing issues #1428/#1430 cover duplicate build-tag overread reports; #1425 covers Linux artifact manifest mismatch.
They are not new findings and are outside this local macOS build's reproduction scope.
#1431 owned piano-roll repeat density at the start and merged during this audit; the exact delta is assessed in context/intervening-changes.md.
Baseline roll-inclusive timings do not measure the new path.

Verdict: retain owner-managed worktrees, one lifecycle build lock per workspace and serialized measurements.
No speculative test-framework or generated-schema replacement.
Native load/reopen, Linux build handoff, dependency migration and CI queue performance remain partial/unexamined by this run.

## Concurrent audit coordination

Otonal contacted this audit during baseline exports, after the initial Harmonigraph build and suite had finished.
Both coordinators agreed to `/private/tmp/yan-deep-audits-expensive.lock` with fcntl exclusive locking around all subsequent builds, profiling and measurements, in addition to each repository lifecycle lock.
An already-running export batch used the earlier Harmonigraph-specific measurement lock; Otonal waited for an explicit process-clear message before beginning its expensive work.
That batch was stopped after unrelated desktop load made the identical 720p export take over 138 seconds without finishing, versus 5.34 seconds in the earlier setup sample.
Only the two identified audit-owned renderer/encoder PIDs were terminated; their whole process tree was verified gone.
The interrupted record is censored evidence of uncontrolled load, not a completed export latency.

The first scratch-probe command narrowed Cargo packages and triggered a different unified feature graph, rebuilding shared dependencies.
That audit-owned attempt was canceled, its compiler children stopped before lock release, and its log preserved.
The replacement uses the same workspace selection as the original release suite.
This is an execution lesson about compatible reuse, not a defect attributed to the project or a comparative build-system result.
