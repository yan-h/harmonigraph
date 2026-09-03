# CLAUDE.md

A Rust CLAP/VST3 plugin that draws a harmonic pitch lattice and an audio
spectrum, plus an offline renderer for video export. Everything below is a
gotcha or a contract — the rest of the repo explains itself by being read.

## Agent guidance has one source

`AGENTS.md` and `GEMINI.md` are symlinks to this file, and `.agents/skills`
is a symlink to `.claude/skills`. Keep shared guidance at those canonical
Claude-facing paths rather than copying it per agent; copies drift while
symlinks make every session read the same contract. Tool-specific hooks,
permissions and commands stay in each tool's native configuration — except
where a Claude path holds procedure rather than settings, which any agent
can read directly: the commands under `.claude/commands/` and the roles they
dispatch under `.claude/agents/`.

## Every change runs in an owner-managed worktree and ends in a draft PR

A session that may change tracked files works in its own worktree, never in
the main checkout. A read-only coordinator may stay in main, but every task it
asks to write gets a separate worktree. The owner determines the path and the
lifecycle:

- **Claude:** `.claude/worktrees/<branch>/`. `EnterWorktree` creates it and
  takes the lock nothing here may take by hand; `reclaim-worktrees.sh` prunes
  and removes it.
- **Codex app:** the app's managed worktree, under `$CODEX_HOME/worktrees` by
  default or its configured Worktree root. Start from the requested committed
  base, normally `main`, and create the requested branch (`codex/<slug>` by
  default) before the first edit because a managed worktree begins detached.
  Stay there through commit, push and draft PR, using the app's approval flow
  for Git metadata and network access. Codex owns its cleanup and snapshots,
  so the Claude reclaimer deliberately leaves it alone.

Keep those ownership domains separate: do not point Codex's Worktree root at
`.claude/worktrees`. A hand-made worktree outside either owner has no automatic
cleanup and is not a supported session workspace. A write-capable session
that finds itself in main leaves whatever is there alone: Claude starts over
through `EnterWorktree`, while a Codex coordinator sends the edit to a
worktree task.

The Claude Companion handoff is not a Codex-managed task: Codex inherits the
dispatching Claude session's cwd. Claude therefore enters its worktree before
dispatching, never after; `.claude/commands/implement-with-codex.md` makes
that its first step.

A completed change is committed, pushed and opened as a **draft** PR with
`gh pr create --draft`, documentation and configuration included; the handoff
says it is open, draft and **not merged**, and nothing merges unless Yan
asks. That is not the whole handoff — a change that touches the picture also
owes the build below, and satisfying one of the two is not satisfying both.

## Lazy-loaded detail lives in `.claude/skills/`

Procedure that only one kind of task needs goes in a skill rather than here.
Every session already carries each skill's description, so reach for the
skill itself; a summary of one in this file is a second copy to maintain.

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
`target/bundled/Harmonigraph.{clap,vst3}`. A branch or worktree build is
invisible in the DAW until its binary is swapped into that slot. With
parallel sessions that slot is shared, so sessions do NOT fight over it — the
model is pull, not push: every session builds into its own worktree, and Yan
chooses which build goes live.

**Sessions: build before you pause; do NOT swap the slot.** Before ending ANY
turn after changing plugin-affecting code — task done, blocked on a question,
partial progress — leave a fresh release build in YOUR worktree so it is
loadable:

```
cargo build --release -p harmonigraph-plugin -p harmonigraph-offline
```

**Both packages, and the second one is the easy half to skip.** The offline
renderer is not a separate picture: it draws through `harmonigraph-ui` and
`harmonigraph-render`, the same crates the editor does, so a change to what
any pane looks like is a change to what an mp4 looks like even when nothing
under `crates/harmonigraph-offline/` is touched. Read "did I touch video
render?" as "did I touch the picture?" and the answer is yes for almost every
change worth loading. It is not a second full build either — the two share
every dependency and the whole UI, so the renderer costs a link on top of a
plugin build that is already done.

What skipping it produces is the quiet failure: `load-plugin.sh` copies
whatever renderer the worktree happens to hold into the one slot the plugin
spawns, so the editor gets the new build and exports keep coming out drawn by
an old one, with nothing on screen saying so. PR #340's lead landed in the
editor and was missing from every render for exactly this reason. The loader
warns when the renderer it is installing predates the branch's HEAD — against
HEAD rather than against the plugin dylib beside it, which is the tempting
comparison and the wrong one, since two artifacts built from one source state
are routinely minutes apart. The warning is a backstop, not the contract —
build both.

Then end your message telling Yan it's `loadable via ./load-plugin.sh
<branch>`, and name the tag the overlay will show (see the `build-handover`
skill). Yan assumes a paused session's change is *built and loadable*, not
that it is already live in the DAW — so the build is the contract, and
touching the shared slot yourself would just evict whatever he is currently
testing. Skip the build only when nothing plugin-visible changed (docs,
backlog, pure-test edits).

**Don't use `cargo xtask bundle` from a nested Claude worktree** — nice-plug-xtask's
`chdir_workspace_root()` takes the *topmost* ancestor with a `Cargo.toml`
(`ancestors().filter(has Cargo.toml).last()`), which for a nested worktree is
the main repo root, so it silently builds main. The bundle looks fresh and
contains none of the branch's changes. `load-plugin.sh` and
`update-plugin.sh` exist to sidestep this.

## House style: comments present-tense and at their own site

**Formatting is not something to think about.** Run `cargo fmt --all`;
`rustfmt.toml` holds the settings and `ci.sh` checks them, so the style is
decided mechanically and a session spends no attention on it. If the output
is ever wrong, change the config rather than hand-formatting around it — a
`#[rustfmt::skip]` where a hand-built table has to keep its columns, and the
tree currently needs none.

Two things the formatter does NOT do. It does not wrap **comment prose**, so
that stays a habit: keep it near 100 columns, where the config puts code. And
it does not touch the `.wgsl` shaders at all.

Don't go looking for a setting for the first one. `wrap_comments` and
`comment_width` exist and would do it, but they are nightly-only, and a
nightly-only key in `rustfmt.toml` is DROPPED with a warning rather than
applied — so on the toolchain `rust-toolchain.toml` pins they do nothing.
What makes this worth writing down is that they look like they work:
`--config wrap_comments=true` on the COMMAND LINE bypasses the channel gate
and reformats 468 hunks, which is not the path `cargo fmt` or `ci.sh` takes.
`imports_granularity` and `group_imports` — one import per line, grouped
std/external/crate, which would be worth having — are behind the same gate.
Buying them means a second pinned toolchain that only rustfmt uses, and
`cargo fmt` then being the wrong command to type.

The two conventions below are the ones still invisible to the build — nothing
fails when you break either, and both are easy to break by reflex.

**Comments state the current constraint, in the present tense.** A comment
describing the delta from a previous version rots: once that version is a
couple of refactors gone it names something no reader can reconstruct, and
it still reads as authoritative, which makes it worse than no comment at
all. Git already holds the history. PR #83 converted 59 such comments
across 23 files, keeping each argument and dropping only its time
reference. Rewrite by keeping the reasoning and dropping the time
reference — state what the ALTERNATIVE would do, not what the code did:
"a window mean rather than the EMA this used to be, which was the wrong
filter" becomes "a window mean rather than an EMA, which IS the wrong
filter".

The exception is where the past tense is load-bearing, and it is a real
category rather than an escape hatch. It is now a SMALL category: the
`legacy_*` fields, the `bare_as_some` shim, the serde aliases and the
historical `default_*` block are all gone (see the compat section below), and
with them most of what used to live here. What is left:

- The **fade param's pre-merge id** (`pitch-class-fade`, in
  `harmonigraph-plugin`'s `lib.rs` and `harmonigraph-take`'s `params.rs`),
  where the id is a live contract with the host's automation lane and the
  rename it outlived is the whole reason it looks wrong.
- The note standing where the retired `node_style` key was, in
  `harmonigraph-scene`'s `view.rs` — the only surviving record of a set of
  seventeen, kept deliberately.
- `DISPLAY_OVERSAMPLE` in `editor.rs`, which carries an explicit
  `HISTORICAL NOTE`: it exists to stop someone tightening the constant on
  reasoning that no longer holds — and is now the only one of its kind.

There the history *is* the current constraint, and flattening it destroys
real information. Runtime "old" and "no longer" — a previously-held voice,
a slab's previously-sent bytes — describe state rather than builds, and are
not in scope at all.

A comment justifying a value by what an OLD BLOB was drawn with is no longer
in the exception; it is now the ordinary rot case, because no code reads an
old blob differently. State what the value is and why, not which build wrote
it.

**A comment states the constraint at its own site; it does not narrate the
code around it.** This is the rule about ALTITUDE, and it is where the
upkeep actually goes: measured over the thirty PRs ending at #437, about
half of every changed line was a comment line and a quarter to a third of
hunks changed nothing BUT comments — and almost none of that churn was
rationale. It was prose mirroring the rest of the system from where it sat:
a doc comment listing the glow's passes ("the light, the moat that takes
light back off, and the cover that..."), rewritten every time a pass came or
went; vocabulary name-checks (moat→standoff, grid→markers,
`grid_at`→`pluses_at`) that cost one edit in code and one per prose
mention; a test's doc comment re-describing the fixture that sits directly
under it. A mirror has to be repainted every time the thing it reflects
moves, and a rename of one word in the picture is paid for at every
sentence that uses it. Concretely:

- Say what THIS value, branch or pass is constrained to and why. If the
  relationship to another site is the constraint, state it in a line and
  link the site (`see X`) — a link does not restate X, so it survives X
  changing; an inventory of X does not.
- Never describe what is ABSENT ("with no moat", "the moat is off here on
  purpose"). Such a comment has a half-life of one PR: the moment the thing
  is deleted, the sentence about its absence is the rot.
- A test's doc comment is the CLAIM — what the measurement shows and why
  that is the thing to measure. The fixture is the code below it; don't
  narrate it.
- A config field's doc carries units, range and what the endpoints mean.
  How the shader or the pane consumes the value belongs at the consumer.

What this does NOT license is cutting rationale. The codebase is heavily
rationale-driven — a comment is often the only carrier of why the code is
weird, and it is the tripwire against a plausible-but-wrong "simplification"
by a reader with no memory of the decision, which here is almost every
reader. Those comments rarely rot, because a constraint does not move when a
neighbour is renamed. The density of the tree (over 40% of the non-blank
lines under `crates/` are comments, doc comments alone nearly 30%) is a COST
this rule exists to stop growing, not a norm to match; a new comment earns
its place by stating a constraint, not by reaching the surrounding average.
Both comment rules are habits to maintain rather than a one-time cleanup —
new PRs regenerate both patterns — and neither is a mandate for a
whole-tree rewrite. Comments carry the rationale, so rewriting them in bulk
is a change of content dressed as a sweep, and no reviewer can read past it
to find the sentences that actually moved.

The same reader is the reason a diff edits the lines that move rather than
reprinting the file around them. Rewriting a file whole to change a few of
its lines spends output on every line that did not move and marks the whole
file as changed, which is the diff nobody can read past. Rewrite whole only
where the file is short or most of it is genuinely moving.

## Two defects that actually ship here: cache keys and fixture reach

Both are cheap to write, invisible to `ci.sh`, and each has landed more than
once. They are the standing prior when reading a diff — your own or anyone's.

**A cache key is wrong in two directions, and the second one is this repo's.**
For every cache, memo, dirty flag or derived value, write down what it is
keyed on and then ask both questions. What else feeds the value and is
missing from the key serves a stale value. What the key carries that decides
nothing about the value is never stale and never still — it churns at the
rate of whatever it should not be watching. The too-wide key is the one with
a record here: a spectrogram column keyed on a whole `SpectrumConfig`
re-uploaded the heatmap on every frame of a drag (`4a4ae66`), and a mark's
key minted fresh per pass held that cache at its eviction limit until a
texture was freed mid-pass (`51d337e`).

A too-wide key is also not merely slow. It restarts the thing it guards often
enough to hide what the carry-forward path gets wrong, so narrowing one is a
change of behaviour rather than of speed: `a2e6e01` is a correctness bug that
was there all along and only became reachable once the key stopped wiping the
evidence every frame. A diff that narrows a key owes an answer for what is
newly reachable.

**A test reaches a path only if its fixture is big enough to get there.** A
fixture too small to reach the new branch passes for the wrong reason and
reads as coverage, which is worse than no test — a green light with nothing
behind it. Issue #450 is the worked example: four shadow tests, each missing
the shape it claims to measure for its own reason, and a disc passing for a
cross through all 145. For a path this diff adds, name the test that executes
it and check the fixture actually arrives.

The count is the other half. A committed test is a file the tree maintains
from then on, so it earns its place the way a comment does: one per
behaviour the task states, sized like the tests already beside it. A scratch
harness or a one-off probe is verification rather than coverage — run it and
read it, and commit it only where something reads it again (see the ISSUE
rule below).

## Backwards compatibility is not a constraint

One or two personal Bitwig projects load this plugin, so a saved blob, a
param range and a recorded automation lane are worth what it costs to reopen
those projects and drag a bar back. Narrow a range, rename a key, drop a
field, move a default: say what breaks in the PR body and make the change.
"This reinterprets saved state" is a line in the description, never a reason
to keep a shape — and never, on its own, a review finding.

What it does not license is a SILENT break. The value on screen must still
be the value the file holds, so a change of range or units carries whatever
clamp or repair keeps the two agreeing (`ViewConfig::sanitize`, and the
`derive_scene` clamps it deliberately leaves to the picture). A blob that
reads out one number while drawing another is a bug at any compat policy.

The tree carries **no compat shims at all**, and that is now the invariant to
hold rather than a state it happens to be in. The `legacy_*` fields, the
`bare_as_some` reader, both `migrate_legacy` passes, the serde aliases for
deleted palettes/orientations/sweep modes, and the `default_*` block whose
job was to keep an old blob from being restyled were all removed at once.
Don't write the next one: a rename is a rename, a dropped variant is dropped.

Two mechanisms carry the weight instead: a container-level
`#[serde(default)]` on every persisted struct, and `UI_PERSIST_VERSION` as a
floor that refuses a blob below it whole rather than half-reading it.
Neither covers a DROPPED ENUM VARIANT, which fails the parse and takes the
entire persist, layout and camera with it — still fine to do, but say so in
the PR body and keep the refusal audible. The `persistence-contract` skill
holds why the floor cannot cover it and where the rule has exceptions; read
it before changing a persisted shape.

## What you could not finish goes to an ISSUE, not the backlog

A session that measures a bug and does not fix it is holding the most
expensive thing it produced: the list of what the bug is NOT. File that
with `gh issue create` — reproduction, what was eliminated and by what
measurement, what was tried and reverted, what is left to try — and link
the PR the probes are in.

A bug you tripped over rather than went looking for takes the same exit. It
is an issue, not a hunk in this diff: a fix riding in on a branch whose
review is about something else gets the least attention of anything in the
PR, and it widens the range `/audit-merges` has to reason about. The
exception is the one that pays for itself — the requested behaviour cannot
work until the bug is fixed — and the PR body says so.

`BACKLOG.md` is not that. An item there is a line of prose, restated at
dispatch and deleted by whoever fixes it, so an investigation parked in it
dies with the fix. It is for things that take five seconds to notice and
need no context to act on.

It is also no longer tracked in git — it is gitignored and per-clone, so a
worktree session has no copy of it and cannot read or add to one. That makes
the issue the only durable channel a session actually has, which sharpens
rather than weakens the rule above.

Issue #121 is the worked example and the reason this is written down: four
hypotheses eliminated by instrumentation across a whole session, and the
first instinct was to compress that into one backlog line. The measurements
are what a future session needs; the symptom it can see for itself.

## Before running sessions in parallel, check for file overlap

Parallelism buys wall-clock only when the work is disjoint; when three
sessions converge on `harmonigraph-ui/src/lib.rs` it buys merge-order bugs
instead, and `/audit-merges` is what pays for them afterwards. Overlapping
work is better run in sequence, and variants of a single decision (three
takes on one fade) are better as one session producing several builds to
compare than as three branches to reconcile.

A Codex coordinator creates a separate top-level app task, and therefore a
separate managed worktree, for each mutating stream. Subagents inside one task
share its worktree; use them for read-only exploration or review, not parallel
edits.

## Never lock an agent-owned worktree by hand

The Claude harness locks its worktree when a session enters it and unlocks it
when the session exits, while Codex owns the lifecycle of its managed
worktree. There is nothing for either session to do here. Every Claude
releaser — both harness exit paths, the startup sweep, and
`.claude/reclaim-worktrees.sh` — recognises a lock only by the shape of its
reason string:

```
claude session <name> (pid <n> start <date>)
```

A reason that does not match that belongs to nobody, and all of them are
right to leave it alone rather than guess at whose it is. That makes a
hand-written lock the one lock here that NOTHING can release: it stands
until a human runs `git worktree unlock`, and while it stands the worktree
is invisible to both tiers of the reclaim script — `target/debug` is never
pruned out of it and the worktree itself is never removable. The instance
that produced this rule pinned 2.2G behind a lock that only a human could
clear (#369).

So don't run `git worktree lock`. If a reason ever does turn up, the string
has to carry `(pid $$ start ...)` in exactly the format above, or it never
comes back.

## Claude permissions a worktree session needs go in `.claude/settings.json`

`.claude/settings.local.json` is gitignored, so a fresh worktree never gets a
copy and every rule in it is inert exactly where most sessions run — a grant
that works in the main checkout still prompts on the branch. Rules that hold
everywhere, the `cargo`/`git`/`gh` workflow, live in the checked-in
`.claude/settings.json`; per-machine paths and one-off grants stay local.
