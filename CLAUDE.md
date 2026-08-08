# CLAUDE.md

A Rust CLAP/VST3 plugin that draws a harmonic pitch lattice and an audio
spectrum, plus an offline renderer for video export. Everything below is a
gotcha or a contract — the rest of the repo explains itself by being read.

## Lazy-loaded detail lives in `.claude/skills/`

- **`build-handover`** — `load-plugin.sh` usage, the `build <branch> @<sha>`
  overlay tag, recovering an evicted build.
- **`capture-daw-state`** — recovering live settings out of a Bitwig project
  (`./read-plugin-state.py`), and the editor-window trap.
- **`pr-hygiene`** — review habit, squash vs merge commit, and the rule for
  when an agent earns a file in `.claude/agents/`.

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
cargo build --release -p harmonigraph-plugin          # add -p harmonigraph-offline if you touched video render
```

Then end your message telling Yan it's `loadable via ./load-plugin.sh
<branch>`, and name the tag the overlay will show (see the `build-handover`
skill). Yan assumes a paused session's change is *built and loadable*, not
that it is already live in the DAW — so the build is the contract, and
touching the shared slot yourself would just evict whatever he is currently
testing. Skip the build only when nothing plugin-visible changed (docs,
backlog, pure-test edits).

**Don't use `cargo xtask bundle` from a nested worktree** — nice-plug-xtask's
`chdir_workspace_root()` takes the *topmost* ancestor with a `Cargo.toml`
(`ancestors().filter(has Cargo.toml).last()`), which for a nested worktree is
the main repo root, so it silently builds main. The bundle looks fresh and
contains none of the branch's changes. `load-plugin.sh` and
`update-plugin.sh` exist to sidestep this.

## House style: hand-formatted, comments in the present tense

Two conventions here are invisible to the build — nothing fails when you
break either, and both are easy to break by reflex.

**Never run `cargo fmt`.** There is no `rustfmt.toml`, and the tree has
never been through rustfmt, so rustfmt's idea of this code and the code
have drifted a long way apart: `cargo fmt --check` currently wants 1346
changes across 82 of the 89 files in `crates/`. Running it once would bury
whatever you actually changed under a whole-tree reformat that no reviewer
can read past, which is why this is a ban rather than a preference. Match
the surrounding style by hand, and wrap only the lines you write — about
100 columns, which is where the tree sits. That is a habit, not a limit to
enforce: 50 lines already exceed it and the longest runs to 155, and
rewrapping code you are only passing through costs a reviewer the same way
`cargo fmt` does, in miniature. To catch only your own long lines, run
`awk 'length>100'` over the lines you added; leave pre-existing ones alone.

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
- The note standing where the retired `node_style` key was, in `view.rs` —
  the only surviving record of a set of seventeen, kept deliberately.
- `DISPLAY_OVERSAMPLE` in `editor.rs`, which carries an explicit
  `HISTORICAL NOTE`: it exists to stop someone tightening the constant on
  reasoning that no longer holds.

There the history *is* the current constraint, and flattening it destroys
real information. Runtime "old" and "no longer" — a previously-held voice,
the ring's previously-written columns — describe state rather than builds,
and are not in scope at all.

A comment justifying a value by what an OLD BLOB was drawn with is no longer
in the exception; it is now the ordinary rot case, because no code reads an
old blob differently. State what the value is and why, not which build wrote
it.

This matters more here than in most repos: comments are ~36% of the non-blank
lines under `crates/`, a quarter of it doc comments, and the codebase is
heavily rationale-driven, so a comment is often the only carrier of why the
code is weird, and it acts as a tripwire against plausible-but-wrong
"simplifications". New PRs keep regenerating the pattern, so this is a habit
to maintain rather than a one-time cleanup.

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

Two mechanisms carry the weight instead, and they are worth keeping straight:

- **Every persisted struct carries a container-level `#[serde(default)]`** —
  `ViewConfig`, `SpectrumConfig`, `RenderConfig`, `RenderFrame`, `Camera`,
  `Gradient`, and each `UiPersist` section but `dock`. `impl Default` is
  therefore the one and only source of a field's fallback: no second set of
  values, and retuning the fresh look is free. A key missing from a blob
  costs that key alone —
  `a_view_missing_any_one_key_reloads_at_the_fresh_value` and
  `a_persist_blob_missing_any_one_section_keeps_the_rest` sweep for it rather
  than pinning one field, because a struct added without the attribute is
  invisible at its declaration. `UiPersist::ui_scale` is the one field-level
  `default = "..."` left, and only because an `f32`'s own default of 0.0 is a
  scale of nothing. Don't add others.
- **`UI_PERSIST_VERSION` is the floor**: a blob below it is refused whole
  rather than half-read. Note what it does NOT cover — see below.

What survives from an old blob is now only what serde gives free: an unknown
KEY is skipped, so retiring a field is safe. An unknown VARIANT is not — it
fails the parse and drops the entire persist, layout and camera with it.

**The floor is no guard against that**, and it is worth being exact, because
it is easy to assume otherwise: the version is read out of a struct that
never parsed, so the check never runs. Raising the floor does nothing for a
dropped variant at any value. What makes it acceptable is that it is LOUD —
`load_persist` returns whether it applied and writes the reason to the
console, the offline renderer prints to stderr, and `a_refused_blob_says_why`
holds both. Dropping an enum variant is still fine; say so in the PR body,
and keep the refusal audible.

## What you could not finish goes to an ISSUE, not the backlog

A session that measures a bug and does not fix it is holding the most
expensive thing it produced: the list of what the bug is NOT. File that
with `gh issue create` — reproduction, what was eliminated and by what
measurement, what was tried and reverted, what is left to try — and link
the PR the probes are in.

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

## Permissions a worktree session needs go in `.claude/settings.json`

`.claude/settings.local.json` is gitignored, so a fresh worktree never gets a
copy and every rule in it is inert exactly where most sessions run — a grant
that works in the main checkout still prompts on the branch. Rules that hold
everywhere, the `cargo`/`git`/`gh` workflow, live in the checked-in
`.claude/settings.json`; per-machine paths and one-off grants stay local.
