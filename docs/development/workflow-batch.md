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
GPU timing implementation is in progress;
Rust checks require sandbox escalation for sccache.
No Bitwig integration has been qualified.
