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

**Sessions: review your own diff before you open the PR.** Not with
`/code-review` — that is a built-in whose frontmatter sets
`disable-model-invocation`, and the harness treats that as locked: no
setting re-enables it, and there is no Bash route to a slash command
either — for the `/code-review ultra` variant, sessions are told in so
many words not to try. It is billed, so it is Yan's to run. The
gate is per-skill and deliberate — `/simplify` sits beside it in the same
built-in family and is model-invocable — so this is a boundary to work
within, not an oversight to work around.

A session's half is therefore the skills it can actually invoke —
`/simplify` (quality only; it does not hunt for bugs) and
`/security-review` — plus a deliberate re-read of `git diff main...HEAD`
as a reviewer rather than as the author. A session has full context on
what it just wrote, which makes this cheap; it is also biased toward its
own work, which is why it is only half the gate. This half catches the
bugs that live entirely inside one branch — a stale invalidation key, an
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

`merge-auditor` does the reading for `/audit-merges` — it hands back candidate
findings and the fix is written in the calling session. That split is the
point: the auditor does not go from "this looks wrong" to a commit without the
failing test in between.

Be precise about how much of that is enforced, because it is easy to read as
more than it is. The agent is granted `Read, Grep, Glob, Bash`: `Write` and
`Edit` are withheld, but **`Bash` writes files, and can commit**. So the split
is instructed, not enforced — the prompt tells it to return findings, and
nothing stops it doing otherwise. `Bash` is granted deliberately: this audit's
findings were proved with `cargo test`, `git merge-base` and `git blame`, and
an auditor that cannot run the suite cannot tell a bug from a guess. Narrowing
`Bash` to read-only patterns would buy enforcement at that price; the trade is
open, not settled.

It is the only agent here, and the rule that keeps it that way is worth
stating: **an agent encodes a job or a constraint, never a description of the
code.** `merge-auditor` describes a method, and a method does not go stale
when a type is renamed.

A `spectral` agent carrying the spectrogram's retention and aggregation
invariants was written and then deleted before it ever ran, which is the
cheaper way to learn the rule. Every fact in it turned out to be already
documented, better, on `SpectrumHistory` and `SpectrogramAgg` — a doc comment
sits in the same diff as the code it describes, so a refactor that invalidates
it puts it in front of the author, and a prompt in `.claude/agents/` has no
such gravity. Duplicating docs into a prompt buys a second copy with worse
invalidation and equal authority.

So: if a fact has a natural home in a doc comment, that is where it goes, and
a session finds it by reading. Reach for an agent when it restricts tools in
a way that changes what can happen, encodes a repeatable job, or isolates
genuinely noisy searching — not when a subsystem merely feels important.
`/audit-merges` still checks the range for drift in whatever agents do exist,
because nothing in the build can catch a prompt that has gone stale.

The corollary is a dispatch rule: **before running sessions in parallel,
check whether they will touch the same files.** Parallelism buys wall-clock
only when the work is disjoint; when three sessions converge on
`lattice-ui/src/lib.rs` it buys merge-order bugs instead, and the audit
above is what pays for them afterwards. Overlapping work is better run in
sequence, and variants of a single decision (three takes on one fade) are
better as one session producing several builds to compare than as three
branches to reconcile.
