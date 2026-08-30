---
description: Hand an implementation task to Codex (gpt-5.6-sol) and keep the brief and the verification here
argument-hint: [--wait] [what Codex should implement]
allowed-tools: Bash, Read, Grep, Glob, AskUserQuestion
---

Delegate the edit to Codex; keep the context assembly and the verification
in this session.

## Why the brief is the whole Claude-side job

Codex reads `AGENTS.md`, which is a symlink to `CLAUDE.md`, so it arrives
knowing the worktree rule, the draft-PR rule and the two-package build. What
it does not know is which file, which line, and which constraint — and the
plugin's `codex:codex-rescue` subagent cannot find out, because its own
definition forbids it from reading the repo. Whatever crosses the handoff
crosses in the prompt.

That is a division worth keeping rather than working around. A brief you can
read before any code exists is where a wrong approach is cheapest to catch.

## First: this session must already be in a worktree

Codex inherits this session's working directory. Dispatched with `--write`
from the main checkout it edits the main checkout, and it cannot rescue
itself from that — the harness lock is not something it can take.

```sh
pwd   # must be under .claude/worktrees/
```

If it is not, **stop and dispatch nothing.** Say so and let Yan enter a
worktree; a session does not move into one mid-flight.

## Writing the brief

Codex has the repository. Send coordinates and constraints, never pasted
source — a brief that quotes the code it is about is paying twice for
something Codex can read for itself, and it goes stale between the read and
the dispatch.

Use the XML block shape the plugin's bundled `gpt-5-4-prompting` skill
prescribes, because that is the dialect the runtime is tuned for:

```
<task>
  What is wrong or missing, at <path>:<line>, and what done looks like.
</task>
<completeness_contract>
  ./ci.sh green. If the picture changed:
  cargo build --release -p harmonigraph-plugin -p harmonigraph-offline
</completeness_contract>
<verification_loop>
  Name the test that executes the new branch, and confirm its fixture is
  large enough to reach it.
  If this adds or narrows a cache key, state what the key is keyed on and
  what is newly reachable once it stops wiping the evidence.
</verification_loop>
<action_safety>
  No unrelated refactors, no comment sweeps. Do not run load-plugin.sh or
  update-plugin.sh. Do not touch anything under .claude/worktrees/.
</action_safety>
```

The `verification_loop` block is where this repo's two standing defect priors
go. They are invisible to `ci.sh`, each has landed more than once, and a
generic coding prompt has no reason to look for either — see the cache-key and
fixture-reach section of `CLAUDE.md`.

The `action_safety` block is not boilerplate either. Codex is one more
Bash-granted agent with the plugin slot in reach, and review subagents have
swapped it before despite explicit prohibitions.

## Dispatch

```sh
CC="$(ls -d ~/.claude/plugins/cache/openai-codex/codex/*/scripts/codex-companion.mjs | sort -V | tail -1)"
node "$CC" task --write --background "<the brief>"
```

Resolve the path rather than hardcoding it: the plugin installs under its own
version number, so a literal path breaks silently on the next update and the
failure looks like Codex being missing.

`--background` unless `--wait` was passed or the task is small and bounded;
an implementation run at `xhigh` is not quick. Poll with `status`, collect
with `result`:

```sh
node "$CC" status --all
node "$CC" result <job-id>
```

Leave `--model` and `--effort` unset. `~/.codex/config.toml` already selects
`gpt-5.6-sol` at `xhigh`, and naming them here means a config change stops
reaching the one caller that most needs it.

Use `task --resume-last` for a follow-up on the same thread, and send only
the delta instruction — restating the brief invites Codex to redo settled
parts of it.

## What comes back here

Read the **diff**, not the files. Re-reading everything Codex touched spends
the context the handoff was meant to save, and it is how a delegated task
ends up costing more than an undelegated one.

Then, before anything else:

```sh
git status                    # nothing outside the worktree
cat target/bundled/.loaded    # the slot is whatever it was
```

Verification stays here because `ci.sh` passing is not the same claim as the
change being right, and Codex has no memory of why the code is weird. What a
review finds here is fixed here — do not send the finding back as a second
task unless the fix is genuinely another implementation job.

Finish the change the way every change finishes: committed, pushed, and open
as a **draft** PR that says what Codex wrote. Yan merges, nothing else does.
