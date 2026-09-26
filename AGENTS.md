# AGENTS.md

A Rust CLAP/VST3 plugin that draws a harmonic pitch lattice and an audio spectrum, plus an offline renderer for video export.
Everything below is a gotcha or a contract —
the rest of the repo explains itself by being read.

## Agent guidance has one source

`CLAUDE.md` and `GEMINI.md` are symlinks to this file, and `.agents/skills` is a symlink to `.claude/skills`.
Edit this file rather than a link:
Claude's Edit and Write tools refuse to write through a symlink.
Cross-project skills are installed globally from the personal `agent-config` checkout ([setup](docs/development.md#shared-agent-skills)).
Keep each skill's guidance at that single source rather than copying it per agent.
Tool-specific hooks, permissions and commands stay in each tool's native configuration —
except `.claude/commands/` and `.claude/agents/`, which hold procedure any agent can read directly.

## Lazy-loaded detail lives in `.claude/skills/`

Procedure that only one kind of task needs goes in a skill rather than here, because this file is paid for by every session.
Every session already carries each skill's description, so reach for the skill itself;
a summary of one in this file is a second copy to maintain.

## Every change runs in an owner-managed worktree and ends in a draft PR

A session that may change tracked files works in its own worktree, never in the main checkout:
Claude in `.claude/worktrees/<branch>/` through `EnterWorktree`;
Codex in its app-managed worktree, creating its `codex/<slug>` branch before the first edit because that worktree begins detached.
A write-capable session that finds itself in main leaves whatever is there alone.
The `worktrees` skill has ownership, the Codex handoff and parallel-session planning.

A completed change is committed, pushed and opened as a **draft** PR with `gh pr create --draft`, documentation and configuration included;
the handoff says it is open, draft and **not merged**, and nothing merges unless Yan asks.
A change that touches the picture also owes the build below, and satisfying one of the two is not satisfying both.

**Never run `git worktree lock`.** Every releaser recognizes only the harness's own reason format, so a hand-written lock stands until a human clears it (#369).

**The merge audit is Yan's to start, and no session's.** When a batch looks worth `/audit-merges`, say so and stop.

## Builds go through sccache

`.cargo/config.toml` sets `rustc-wrapper = "sccache"`, so **`sccache` must be on PATH or every build dies with "could not execute process sccache"** (`brew install sccache`).
`RUSTC_WRAPPER="" cargo build ...` bypasses it to rule it out.
Each worktree keeps its own `target/`;
the cache shares compiled dependencies between them.

## Pausing = a loadable build exists (sessions build, Yan loads)

Before ending ANY turn after changing plugin-affecting code —
task done, blocked on a question, partial progress —
leave a fresh release build in YOUR worktree, and do NOT swap the shared DAW slot:

```
cargo build --release -p harmonigraph-plugin -p harmonigraph-offline
```

**Both packages.** The offline renderer draws through `harmonigraph-ui` and `harmonigraph-render`, so any picture change is a video-export change even when nothing under `crates/harmonigraph-offline/` moved;
skipping it leaves exports drawn by an old binary with nothing on screen saying so (PR #340).
Push and open the PR first, then build while CI runs.
End by telling Yan it's loadable via `./load-plugin.sh <branch>` and naming the overlay tag it will show.
A session on a remote machine (a cloud session) has no local worktree for the loader to find, so it hands over `./build-pr.sh --load <PR>` instead.
Skip the build only when nothing plugin-visible changed (docs, backlog, pure-test edits).
The `build-handover` skill has the loader, the tag and why not `cargo xtask bundle`.

## A shader-affecting edit owes a regenerated Metal corpus in the SAME commit

`crates/harmonigraph-metal-assets/assets` is keyed on the generated MSL and compiler options, not the `.wgsl` text:
a pipeline-layout change in Rust, a `vendor/wgpu-hal` bump or a `Cargo.lock` move invalidates it with no `.wgsl` in the diff, and a comment-only `.wgsl` edit does not.
Tests, fmt and clippy all pass against a stale corpus;
this gate does not, in about fifteen seconds:

```
HARMONIGRAPH_SHADER_ASSETS=strict cargo test -p harmonigraph-render \
  --features shader-assets-tools -- --ignored --exact \
  shader_assets::catalog::production_metal_asset_catalog
```

`Full CI` green is not mergeable: read `mergeStateStatus`, because `Metal shader assets` reports separately.
Regenerate on the runner, never locally —
the `metal-corpus` skill has the command, the import, and why grepping the fallback notice is not a check (#947).

## House style: formatting is mechanical, and a diff edits what moves

Run `cargo fmt --all`;
`rustfmt.toml` holds the settings (and why the nightly-only comment-wrapping keys are absent), and `ci.sh` checks them.
If the output is ever wrong, change the config rather than hand-formatting around it.
The local pre-push gate is only `cargo fmt --all --check`;
`ci.sh` runs in GitHub Actions.

Markdown prose is laid out one clause per line instead of being wrapped to a width;
`.claude/semantic-breaks.py --write` does it and `ci.sh` gates it.

A diff edits the lines that move rather than reprinting the file around them.
Rewrite whole only where the file is short or most of it is genuinely moving.

## Two defects that actually ship here: cache keys and fixture reach

Both are cheap to write, invisible to `ci.sh`, and each has landed more than once.
They are the standing prior when reading a diff —
your own or anyone's.

**A cache key is wrong in two directions, and the second one is this repo's.** For every cache, memo, dirty flag or derived value, write down what it is keyed on and then ask both questions.
What else feeds the value and is missing from the key serves a stale value.
What the key carries that decides nothing about the value is never stale and never still —
it churns at the rate of whatever it should not be watching.
The too-wide key is the one with a record here:
a spectrogram column keyed on a whole `SpectrumConfig` re-uploaded the heatmap on every frame of a drag (`4a4ae66`), and a mark's key minted fresh per pass held that cache at its eviction limit until a texture was freed mid-pass (`51d337e`).

A too-wide key is also not merely slow.
It restarts the thing it guards often enough to hide what the carry-forward path gets wrong, so narrowing one is a change of behavior rather than of speed:
`a2e6e01` is a correctness bug that was there all along and only became reachable once the key stopped wiping the evidence every frame.
A diff that narrows a key owes an answer for what is newly reachable.

**A test reaches a path only if its fixture is big enough to get there.** A fixture too small to reach the new branch passes for the wrong reason and reads as coverage, which is worse than no test.
Issue #450 is the worked example:
four shadow tests, each missing the shape it claims to measure, and a disc passing for a cross through all 145. For a path this diff adds, name the test that executes it and check the fixture actually arrives.

The count is the other half.
A committed test earns its place the way a comment does:
one per behavior the task states, sized like the tests already beside it.
A scratch harness or a one-off probe is verification rather than coverage —
run it and read it, and commit it only where something reads it again.

## Backwards compatibility is not a constraint

One or two personal Bitwig projects load this plugin, so a saved blob, a param range and a recorded automation lane are worth what it costs to reopen those projects and drag a bar back.
Narrow a range, rename a key, drop a field, move a default:
say what breaks in the PR body and make the change.
"This reinterprets saved state" is a line in the description, never a reason to keep a shape —
and never, on its own, a review finding.

What it does not license is a SILENT break.
The value on screen must still be the value the file holds, so a change of range or units carries whatever clamp or repair keeps the two agreeing (`ViewConfig::sanitize`, and the `derive_scene` clamps it deliberately leaves to the picture).

The tree carries **no compat shims at all**, and that is the invariant to hold:
no `legacy_*` fields, migration passes or serde aliases for deleted variants.
A rename is a rename, a dropped variant is dropped.
A container-level `#[serde(default)]` on every persisted struct and the `UI_PERSIST_VERSION` floor carry the weight instead;
neither covers a DROPPED ENUM VARIANT, which fails the whole parse —
still fine, but say so in the PR body.
Read the `persistence-contract` skill before changing a persisted shape.

## What you could not finish goes to an ISSUE, not the backlog

A session that measures a bug and does not fix it is holding the most expensive thing it produced:
the list of what the bug is NOT.
File that with `gh issue create` —
reproduction, what was eliminated and by what measurement, what was tried and reverted, what is left to try —
and link the PR the probes are in (issue #121 is the worked example).

A bug you tripped over rather than went looking for takes the same exit, not a hunk in this diff:
a fix riding in on an unrelated branch gets the least review attention of anything in the PR.
The exception is when the requested behavior cannot work until the bug is fixed, and the PR body says so.

`BACKLOG.md` is not the alternative:
it is gitignored and per-clone, so a worktree session has no copy of it.

## Claude permissions a worktree session needs go in `.claude/settings.json`

`.claude/settings.local.json` is gitignored, so a fresh worktree never gets a copy and its rules are inert exactly where most sessions run.
Rules that hold everywhere live in the checked-in `.claude/settings.json`;
per-machine paths and one-off grants stay local.
