# CLAUDE.md

## Builds go through sccache

`.cargo/config.toml` sets `rustc-wrapper = "sccache"`, so **`sccache` must be
on PATH or every build dies with "could not execute process sccache"**
(`brew install sccache`). To rule it out as the cause of a build failure,
`RUSTC_WRAPPER="" cargo build ...` bypasses it.

Each worktree still keeps its own `target/` — parallel sessions never
serialise on a shared target lock, they only share compiled dependencies. That
is what the cache is for: a release build in a brand-new worktree takes 1m28s
instead of 3m36s. `sccache --show-stats` reports the hit rate.

## Pausing = a loadable build exists (sessions build, Yan loads)

Bitwig loads exactly ONE plugin build: the main checkout's
`target/bundled/MIDI Lattice 3D.{clap,vst3}`. A branch or worktree build is
invisible in the DAW until its binary is swapped into that slot. With
parallel sessions that slot is shared, so sessions do NOT fight over it — the
model is pull, not push: every session builds into its own worktree, and Yan
chooses which build goes live.

**Sessions: build before you pause; do NOT swap the slot.** Before ending ANY
turn after changing plugin-affecting code — task done, blocked on a question,
partial progress — leave a fresh release build in YOUR worktree so it is
loadable:

```
cargo build --release -p midi_lattice_3d          # add -p lattice-offline if you touched video render
```

Then end your message telling Yan it's `loadable via ./load-plugin.sh
<branch>`. Yan assumes a paused session's change is *built and loadable*, not
that it is already live in the DAW — so the build is the contract, and
touching the shared slot yourself would just evict whatever he is currently
testing. Skip the build only when nothing plugin-visible changed (docs,
backlog, pure-test edits).

**Yan: load whichever build you want.**

- `./load-plugin.sh` — menu of every worktree's build (freshness + which one
  is live now); pick a number to swap it in.
- `./load-plugin.sh <branch>` — load that branch's build directly (unique
  substring is fine).
- `./load-plugin.sh --list` — just print the table, load nothing.

It copies only, never builds; a build must already exist in the worktree.
Stale builds (dylib older than the branch's HEAD) are flagged but still
loadable. After a swap, rescan/restart the plugin in Bitwig to pick it up.

- Both `load-plugin.sh` and `update-plugin.sh` record the live build in
  `target/bundled/.loaded`, so "what's loaded?" is answerable without guessing.
- `./update-plugin.sh` remains the build-and-load-in-one-shot path (it builds
  the checkout it runs FROM and swaps that immediately) — use it when you
  explicitly want a session to make its own build live, e.g. a single-session
  flow. Run it from the main checkout and it rebuilds main, not your branch.
- Don't use `cargo xtask bundle` from a nested worktree — it resolves the
  topmost workspace and builds main. These scripts exist to sidestep this;
  see `update-plugin.sh`'s header comment.

### Every build says which build it is

The performance overlay's bottom line reads `build  <branch> @<sha>` — the
branch with its `worktree-` prefix stripped, so it is exactly the argument
`./load-plugin.sh <branch>` takes. It is stamped at compile time by
`crates/lattice-ui/build.rs` and is on by default, so it needs nothing from
Yan but a look at the corner of the Analyzer pane.

This exists because a swap can silently not have happened: no rescan, a build
that landed in a different worktree, the wrong branch named, or a build that
never finished. Two builds are otherwise indistinguishable from inside the
DAW, and a look that is judged against the wrong binary costs a whole round
trip to discover.

**Sessions, when you hand over a build: say what tag it will show.** Not
"loadable via `./load-plugin.sh <branch>`" alone — name the tag too, so the
first thing Yan can do is confirm the swap took. It is
`<branch minus worktree- prefix> @<short sha of your last commit>`; `git log
--oneline -1` gives you the sha. This matters most when you hand over MORE
THAN ONE build to compare (variants of a look, an A/B of a fix): with several
near-identical builds in play, "which one am I looking at?" is the whole
question, and the tag is the only answer that cannot be fooled.

The tag names the last COMMIT, not the working tree — a build made with
uncommitted edits carries the commit it sits on, exactly as
`load-plugin.sh`'s freshness column does. So commit before you build if you
want the tag to distinguish your work.

## Reading the plugin's live settings back out of Bitwig

When Yan has dialed in a look in the DAW and wants it captured (new
`ViewConfig::default()`, a bug reproduced against real state), don't guess
and don't read numbers off a screenshot — the exact values are recoverable:

```sh
./read-plugin-state.py            # newest project: params, camera, view
./read-plugin-state.py --rust     # view fields as an impl Default body
```

**The trap, which costs a round trip with Yan every time it's missed:** the
UI state (dock, camera, ViewConfig) is written into the plugin state ONLY
when the editor WINDOW is closed (`impl Drop for LatticeEditorHandle`,
`crates/midi_lattice_3d/src/editor.rs`). Saving a project with the plugin
window open silently keeps the previous values. So ask Yan for, in order:

1. close the MIDI Lattice 3D **window**, then
2. save the project (Cmd+S).

Only then run the script. Host-automatable params (tuning, fade, color
range) are exempt — they live in the param system and are always current,
which is why a project can show fresh params next to a missing `ui-state`.

The script explains the container format in its header. Projects live under
Google Drive, not `~/Documents/Bitwig Studio/Projects` (empty); it finds
them via `mdfind`.

## Review happens at the merge boundary, not on the branch

Nothing mechanical blocks a merge here: GitHub Actions is disabled on the
repo and branch protection is not available on this plan, so `ci.sh` via
the `.githooks/pre-push` hook is the only automatic gate, and it checks
clippy and tests — not judgement. Review is therefore a habit, in two
halves, and each half catches a class the other cannot.

**Sessions: review your own diff before you open the PR.** Run
`/code-review` and act on what it finds. A session has full context on what
it just wrote, which makes this cheap; it is also biased toward its own
work, which is why it is only half the gate. This half catches the bugs
that live entirely inside one branch — a stale invalidation key, an
underflow, a test whose fixture never reaches the new path.

**Yan: run `/audit-merges` after a batch of merges lands.** Parallel
sessions produce branches that are each correct against the `main` they
started from, so the interesting bugs are the ones that do not exist until
two of them are combined — and a per-branch review is structurally blind to
those. PR #85 is the worked example: 12 PRs merged in one night, two real
bugs, both of them a cache whose missing input arrived in a *different* PR.
The command reads the combined diff and keeps a `last-merge-audit` tag so
consecutive audits do not re-read the same range.

### The agents in `.claude/agents/`

`merge-auditor` does the reading for `/audit-merges` — read-only, so it hands
back candidate findings and the fix is written in the calling session. That
split is the point: a read-only auditor cannot skip from "this looks wrong"
to a commit without the failing test in between.

`spectral` carries the retention and aggregation invariants of the
spectrogram path, which is the subsystem that has shipped the same bug class
twice. It states what is stable (`lattice-core`) and points at what is moving
(the pane) rather than quoting it, because a prompt that quotes volatile code
goes stale silently and is then believed.

**An agent prompt is a cache of facts about the code, so it needs an
invalidation key like any other.** Here that is two habits: a PR changing a
subsystem updates the agent that describes it, in the same PR; and
`/audit-merges` checks the range for that drift and reports it as a finding.
Nothing in the build can catch a stale prompt, so it has to be caught by the
same pass that catches stale caches.

The corollary is a dispatch rule: **before running sessions in parallel,
check whether they will touch the same files.** Parallelism buys wall-clock
only when the work is disjoint; when three sessions converge on
`lattice-ui/src/lib.rs` it buys merge-order bugs instead, and the audit
above is what pays for them afterwards. Overlapping work is better run in
sequence, and variants of a single decision (three takes on one fade) are
better as one session producing several builds to compare than as three
branches to reconcile.
