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
have drifted a long way apart: `cargo fmt --check` currently wants 835
changes across 51 of the 57 files in `crates/`. Running it once would bury
whatever you actually changed under a whole-tree reformat that no reviewer
can read past, which is why this is a ban rather than a preference. Match
the surrounding style by hand, and wrap only the lines you write — about
100 columns, which is where the tree sits. That is a habit, not a limit to
enforce: 54 lines already exceed it and the longest runs to 155, and
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
category rather than an escape hatch. Comments about **blobs still sitting
in saved projects** must stay historical — the `default_*` block and
`migrate_legacy` in `view.rs`, the serde aliases for deleted palettes and
node styles, the `low_octave` sentinel, the version-0 dock refresh — and so
must the fade param's pre-merge id, where the id is a persistence contract
that outlives the rename. There the history *is* the current constraint,
and flattening it destroys real information. `DISPLAY_OVERSAMPLE` in
`editor.rs` carries an explicit `HISTORICAL NOTE` on the same grounds: it
exists to stop someone tightening the constant on reasoning that no longer
holds. Runtime "old" and "no longer" — a previously-held voice, the ring's
previously-written columns — describe state rather than builds, and are not
in scope at all.

This matters more here than in most repos: the codebase is ~23% comments and
heavily rationale-driven, so a comment is often the only carrier of why the
code is weird, and it acts as a tripwire against plausible-but-wrong
"simplifications". New PRs keep regenerating the pattern, so this is a habit
to maintain rather than a one-time cleanup.

## Review, and what a session may not invoke

`ci.sh` via `.githooks/pre-push` is the only automatic gate, and it checks
clippy and tests — not judgement. So: **sessions run `/self-review` before
opening a PR**; Yan runs `/audit-merges` after a batch of merges lands.

**`/code-review` and `/code-review ultra` are Yan's to run — a session must
not try to invoke them.** They are billed, and the built-in sets
`disable-model-invocation`, which the harness treats as locked. There is no
Bash route either. Details of both halves, and the squash rule, are in the
`pr-hygiene` skill.

## Before running sessions in parallel, check for file overlap

Parallelism buys wall-clock only when the work is disjoint; when three
sessions converge on `harmonigraph-ui/src/lib.rs` it buys merge-order bugs
instead, and `/audit-merges` is what pays for them afterwards. Overlapping
work is better run in sequence, and variants of a single decision (three
takes on one fade) are better as one session producing several builds to
compare than as three branches to reconcile.
