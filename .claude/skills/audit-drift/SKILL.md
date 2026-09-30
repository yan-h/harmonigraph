---
name: audit-drift
description: Audit one area of the product for drift from Yan's intent — machinery whose reason has gone, frozen values that stopped fitting, tests guarding a mechanism instead of a behaviour, prose describing code that no longer exists. Started only by Yan invoking it explicitly, with an area.
disable-model-invocation: true
---

# Audit one area for drift from intent

**Yan starts this.** A session that notices drift says the area looks worth auditing and stops there;
it does not run the procedure below by hand instead.

`disable-model-invocation` above enforces that in Claude.
Codex ignores it and reaches a skill by reading this file, so in Codex these lines are the rule itself.

## What drift is

Code that no longer serves Yan's intent although every change on the way was locally reasonable.
It is a different axis from the merge audit:
not a bug born where two branches meet, but a reason that moved while the code it justified stayed.
The worked example is #1285:
a glow-dimming mechanism added for note names (#522), extended to nodes by analogy (#535), frozen at full strength (#537),
then taken off names when they got a better method (#1164, "geometry shadows keep their existing compositing") —
leaving nodes with a mechanism no reason still held up, and a Darkness slider that couldn't reach zero.

The kinds seen so far, each with a record here:

- **Orphaned mechanism.** Its motivating case was removed or replaced; it stays for the others (#1285, #1289, #1290, #1292).
- **Frozen or captured value that stopped fitting** once something beside it moved (#537, #934, #1159).
- **Test guarding a mechanism rather than a behaviour, or a fixture that cannot reach its case** — they pass while defending the leftover (#1285, #1272, #1292).
- **Intent restated wrongly, then built on** — a handoff or cleanup keeps the data and loses the model (#1130, #1278 → #1284).
- **Prose describing something removed** (#1291, #1292).

The common signal is a removal or replacement PR saying WHAT it keeps ("keep existing", "preserve", "unchanged", "comments only") and not WHY the kept part still has a reason.

## Choose the area

The area is the text after the invocation (`$0` in Claude, the text after `$audit-drift` in Codex):
a pane or feature as Yan names it — "ribbons and their bloom", "lattice shadows".
With none, ask for one; a whole-repo pass is too broad to verify.

Read `docs/intent/<area>.md` if it exists: Yan's recorded answers from earlier audits of this area.
They are the one place his picture of the area is written down, and a finding that contradicts one is the strongest kind.

## Run two agents in parallel

Both read-only, both on `origin/main`, both told never to build a plugin slot or run `load-plugin.sh`.
Pass each its brief from `references/` and the area.

- **Describer** (`references/describer.md`): writes how the area BEHAVES from the current code alone —
  no history, no PRs, no issues, no notes.
  Blind on purpose: in the first trial it found the known live case (#1289) with none of the history the auditor had.
- **Auditor** (`references/auditor.md`): traces the area's mechanisms to the PRs that added them and asks whether each reason still holds.

A capable model at high effort for the auditor; the describer is lighter work.
The first trial cost about 190k tokens for the describer and 275k for the auditor, twelve minutes of wall clock.

## Verify, then filter

Verify every finding against `origin/main` before it goes anywhere;
the auditor's line numbers drift and its "dead" can be a caller it missed.

Then split what survived:

- **Mechanical** — nothing on screen changes, or the fix follows from a decision already on record:
  one grouped issue per area (#1292 is the shape), skipping anything an open issue already covers.
- **Needs Yan's call** — the behaviour may or may not be what he wants.
  A describer item qualifies only when the auditor's history shows its reason moved, a recorded decision contradicts it, or it has a consequence Yan would notice (an export differing from the live pane, a control that cannot reach its end).

**Never hand Yan the raw description.** A forty-item page was tiring and came back mostly "fine" or "don't care" (the 2026-09-30 trial).
Bring at most five questions, each answerable yes or no, each with what you would do on either answer.

## Record the answers

Each answer Yan gives goes into `docs/intent/<area>.md`, one line per decision:
the behaviour, his call, the date and the issue it produced.
A "yes, that's fine" is worth recording too — it is what stops the next audit asking again.
Nothing he did not answer goes in; this file is his picture, not the describer's.

Commit it with the audit's issues listed in a draft PR, per `AGENTS.md`.
