# Workflow batch implementation notes

This batch implements #1388, #1381, #1390, #1391, #1395 and #1394 on `codex/workflow-batch`.
The owner authorized autonomous review and merge after Full CI and CLEAN merge status;
manual Bitwig testing is not a merge prerequisite.
One managed worktree and one writer own the implementation.
Independent agents explore and review without editing.
Do not run audit-merges or swap the shared plugin slot.

## Starting evidence

Current main is `807be883`.
All six issues were open with no comments when work began.
Open PRs #1353, #1321 and #1322 do not implement this batch;
#1353 changes nearby visual controls and remains separate.

## Sequence and verification

1. #1388: optional consume-once GPU completion through renderer, window queue and metrics.
2. #1381: worker-side exclusive file creation and collision retries, including later passes.
3. Settle shared state boundaries, then undo/redo and saved looks/A-B.
4. Host-owned camera movement and recorded transport-time replay.
5. Sequential export queue with captured settings and per-job cancellation/retry.
6. Independent combined review, confirmed fixes, all CI gates, ready and merge.
7. Build both release packages at the merged result and read the actual overlay tag.

## Progress

Initial skills read: worktrees, pr-hygiene, persistence-contract, build-handover.
All six changes are implemented on the integration branch.
Independent combined review found no remaining correctness defect or necessary design change.
Rust checks require sandbox escalation for sccache.
No Bitwig integration has been qualified.

## State ownership decision

The live appearance remains `PictureState.appearance`.
Host parameters own camera movement and tuning;
the editor appearance fields are a synchronized drawing/saving snapshot of movement.
Camera projection and Cabinet shear are static appearance.
Settings history, A/B and named looks include static camera geometry, visual view and spectrum settings.
They exclude movement, tuning mirrors, editor chrome/layout and output settings.
Existing automatable fade and color endpoints remain under host undo too.
This avoids undo or recall unexpectedly replacing incoming automation.
Named looks are project/instance-local, matching saved camera angles.
History is transient and bounded to 64 actions per comparison slot.
Queued exports capture appearance and output at enqueue;
recorded movement overrides corresponding movement channels in the selected appearance.
Old takes without movement channels retain the selected appearance camera.
Recorded parameter evaluation uses the existing block-timestamp step semantics.
The export queue lasts for the plugin instance, survives editor closure, and has no crash resume.

## Correctness fix evidence

#1388 consumes completed GPU measurements once.
All 16 perf tests and the patched vendored queue test pass;
plugin check passes.
Independent review found no correctness defect.
#1381 owns exclusively created recording file pairs.
All 100 recorder tests passed, including simultaneous exclusive pair creation and failure cleanup.
All 28 take tests passed after fixing the validation-order regression caught by the existing no-truncation test and independent review.
Obsolete partial-initialization failure branches were removed;
optional audio ownership remains necessary for finalization.
PR #1397 is the integration PR; its GitHub state is authoritative.

## Appearance and camera evidence

Undo/redo and A/B plus saved looks are separate logical changes.
Six appearance tests and 51 persistence tests pass.
The continuous drag test drives the real slider;
commands are deferred until numeric focus loss commits its final value.
Each A/B slot owns a separate transient history.
Undo buttons operate locally;
Cmd/Ctrl-Z remains the host's shortcut because the native shell forwards it to the DAW.

Five camera parameters own yaw, pitch, distance and absolute lattice X/Y pan.
Cabinet uses pan and distance;
yaw/pitch remain available for returning to other projections, while Cabinet shear stays static.
The real CLAP harness verifies two-axis gestures, saved host parameter restoration,
recording with no editor at mid-song, and fresh camera baselines after a loop wrap.
Offline tests verify recorded movement overrides a replacement appearance at differing frame cadences.
Old takes without camera channels retain their selected appearance camera.
Camera capture is block-rate: the last event within one audio callback is stamped at that block's origin.
These tests do not establish Bitwig lane writing, touch/latch behavior or host gesture UX.

## Queue implementation and verification

The Video pane queues the last take or newline-separated take paths.
Each job captures current, named or recorded appearance plus the current Video settings at the button press.
Named looks apply to a clone;
subsequent undo, A/B, library changes, output changes and incoming host camera values cannot rewrite a pending job.
Take replay still owns recorded camera movement.
Automatic recording completion also appends a job.
The queue is per plugin instance, sequential and started on demand;
closing the editor preserves it, while plugin destruction cancels pending/active jobs and joins the renderer worker.
There is no crash resume or global background service.

Each export exclusively creates a scratch directory beside its take.
Successful publication uses a same-filesystem hard link and numbered filenames;
existing files, including dangling symlinks, are never replaced.
Cancellation kills the renderer's private process group, waits for it to exit and cleans only owned scratch files.
Failed and cancelled jobs can be retried with their captured settings.
Finished entries remain visible until explicitly cleared.

Real subprocess fixtures cover FIFO variants, distinct captured sizes/looks, a transient failure followed by success,
retry, pending/active cancellation, inherited stderr descendants, cancellation just before publication,
plugin-shutdown queue closure, independent instances, and output collisions.
Independent queue review found a dangling-symlink retry loop;
a filesystem probe confirmed it and the regression fixture now reaches that case.
UI regression sweeps caught toolbar width expansion;
wrapping controls and a compact Looks header fix it, and nonappearance pages no longer carry irrelevant appearance controls.
A combined interaction test checks queued snapshots across undo, A/B, saved-look deletion and incoming camera values.

Existing projects predating camera parameters use the new host parameter defaults for movement;
the old camera fields in the UI blob are no longer authoritative in the plugin.
This is an intentional saved-state change without compatibility shims.
No Bitwig project was modified and the shared DAW slot was not swapped.

Merge gate: resolve confirmed findings, fold corrective revisions into their logical commits,
wait for all checks including reported Full CI and CLEAN merge status, then mark ready and merge.
Build both release packages at the merged result before handing over the load command.

The expanded Looks and Exports controls are exercised through real pointer events at a 120-point column width.
That fixture exposed a default combo-box width overflow;
bounded text entry and truncated selection labels repair it without expanding controls below it.
