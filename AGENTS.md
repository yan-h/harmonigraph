# AGENTS.md

A Rust CLAP plugin that draws a harmonic pitch lattice and an audio spectrum, plus an offline renderer for video export.

## Agent guidance has one source

`CLAUDE.md` and `GEMINI.md` are symlinks to this file, and `.agents/skills` is a symlink to `.claude/skills`.
Edit this file rather than a link:
Claude's Edit and Write tools refuse to write through a symlink.
Cross-project skills are installed globally from the personal `agent-config` checkout ([setup](docs/development.md#shared-agent-skills));
keep each skill's guidance at that single source rather than copying it per agent.
Tool-specific hooks, permissions and commands stay in each tool's native configuration —
except `.claude/agents/`, which holds procedure any agent can read directly.

## Lazy-loaded detail lives in `.claude/skills/`

Procedure that only one kind of task needs goes in a skill rather than here, because this file is paid for by every session.
Every session already carries each skill's description, so reach for the skill itself;
a summary of one in this file is a second copy to maintain.

## Every change runs in an owner-managed worktree and ends in a draft PR

A session that may change tracked files works in its own worktree, never in the main checkout:
Claude in `.claude/worktrees/<branch>/` through `EnterWorktree`;
Codex in its app-managed worktree, creating its `codex/<slug>` branch before the first edit because that worktree begins detached.
A write-capable session that finds itself in main leaves whatever is there alone.

A completed change is committed, pushed and opened as a **draft** PR with `gh pr create --draft`, documentation and configuration included;
the handoff says it is open, draft and **not merged**, and nothing merges unless Yan asks.
A change that touches the picture also owes the build below, and satisfying one of the two is not satisfying both.

**Never run `git worktree lock`.** Every releaser recognizes only the harness's own reason formats, so a hand-written lock stands until a human clears it (#369).

**The merge audit is Yan's to start, and no session's.** When a batch looks worth `/audit-merges`, say so and stop.

## Builds go through sccache

`.cargo/config.toml` sets `rustc-wrapper = "sccache"`, so **`sccache` must be on PATH or every build dies with "could not execute process sccache"** (`brew install sccache`).
`RUSTC_WRAPPER="" cargo build ...` bypasses it to rule it out.

## Pausing = a loadable build exists (sessions build, Yan loads)

Before ending ANY turn after changing plugin-affecting code —
task done, blocked on a question, partial progress —
leave a fresh release build and a preserved handoff from YOUR worktree, and do NOT swap the shared DAW slot:

```
cargo build --release -p harmonigraph-plugin -p harmonigraph-offline
```

**Both packages.** The offline renderer draws through `harmonigraph-ui` and `harmonigraph-render`, so any picture change is a video-export change even when nothing under `crates/harmonigraph-offline/` moved;
skipping it leaves exports drawn by an old binary with nothing on screen saying so (PR #340).
Skip the build only when nothing plugin-visible changed (docs, backlog, pure-test edits).

Push and open the draft PR first, then run `./session-lifecycle.sh handoff`, which builds both packages while CI runs.
A plain `handoff` records the work as complete and trims `target/`,
so a pause mid-iteration runs `handoff --keep-cache` instead, or the build above while source is still uncommitted.
End by telling Yan it's loadable via `./load-plugin.sh <branch>` and naming the overlay tag it will show, read with `./load-plugin.sh --tag`.
A session on a remote machine (a cloud session) has no local worktree for the loader to find, so it hands over `./build-pr.sh --load <PR>` instead.
It skips `handoff` as well:
the preserved artifact is the macOS `.dylib`, so on Linux `handoff` refuses after a successful build,
and a Linux `.so` has no DAW to load it (#1425).

## House style: formatting is mechanical, and a diff edits what moves

Run `cargo fmt --all`;
`rustfmt.toml` holds the settings (and why the nightly-only comment-wrapping keys are absent), and `ci.sh` checks them.
If the output is ever wrong, change the config rather than hand-formatting around it.
Before pushing Rust, run `./ci.sh quick` (the opt-in pre-push hook runs it too):
formatting, prose, clippy and rustdoc, well under a minute once the worktree has built,
the failures that otherwise each cost a ~10 minute CI round.
The full `ci.sh` runs in GitHub Actions, and reports every gate that fails rather than the first.

Markdown prose is laid out one clause per line instead of being wrapped to a width;
`.claude/semantic-breaks.py --write` does it and `ci.sh` gates it.

A diff edits the lines that move rather than reprinting the file around them.
Rewrite whole only where the file is short or most of it is genuinely moving.

A change to what a field or function returns renames it, so the compiler lists every reader;
#1686 did this for the two presence levels after a changed meaning had left one reader untraced (#1316).

## Two defects that actually ship here: cache keys and fixture reach

Both are cheap to write, invisible to `ci.sh`, and each has landed more than once,
so they are the standing prior when reading any diff, your own included.

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
four shadow tests, each missing the shape it claims to measure, and a disc passing for a cross through all 145.
For a path this diff adds, name the test that executes it and check the fixture actually arrives.

The count is the other half.
A committed test earns its place the way a comment does:
one per behavior the task states, sized like the tests already beside it.
A scratch harness or a one-off probe is verification rather than coverage —
run it and read it, and commit it only where something reads it again.

**A test builds the inputs its assertion depends on.** When the shipped default or layout is itself the subject, it asserts a property of whatever is there rather than the value there today.
A test that drives the real UI by bar label or position, or inherits the fresh view's look, breaks on every defaults capture and layout move:
the guard-ledger audit (#1709) found the top tenth of the regression tests absorbing nearly half the edits that intended changes forced,
and #836, #1107, #1112 and #1714 each paid that down one test at a time.
A neighbouring test is a model for size, not for where its inputs come from.

## The stakes record: one composer on macOS in Bitwig, and three failures that are severe

Price a case from this section, as the global preferences say:
how often it occurs here, and what it costs when it does.

**Use.** Yan alone, composing in Bitwig Studio on macOS 14 or later, on Apple silicon with a 144 Hz display,
and exporting the pieces as videos.
The projects run at 44.1 or 48 kHz (Yan, 2026-11-14).
One or two personal Bitwig projects hold saved state.
Anyone else builds from source with rough edges promised (README), and no case is handled for their sake.
Linux is only where cloud agent sessions build and test.
Nobody runs the plugin on Linux, X11 or Windows, so a case only those platforms reach is outside the use.

**Normal load.** A chord of about six tones with a handful of held notes, inside the caps of 64 held voices per source and 256 per session;
an editor drawing at up to 144 Hz;
sessions of tens of minutes;
exports of a minute or two at 1440p60 or 720p60, never over the ten-minute cap.
A figure that appears only in a test or a review finding is not load:
24 held notes at 2048², or more than 2,048 events in the callback that stops a take.

**Severe failures, and the parts they live in.**

- Saved editor state (the look, slots, presets and undo) lost or changed with nothing saying so (#1740, PR #1764).
- A video export, or the take behind it, that is wrong with nothing saying so (PR #340).
- Time taken from Bitwig's main thread or from the audio thread:
  a still picture once cost the main thread 280-320 ms a second (#1673).

Everything else is cheap:
a warning, a glitch for a frame, a bar dragged back after reopening a project, an old take that no longer loads.

**A case outside the use** is refused, or falls back to the default, with a line in the Console (`harmonigraph-ui/src/state.rs`).
One that no producer writes is a `debug_assert` and not a release workaround (#1631).
Neither gets a handler or a repair of its own.
The per-type `sanitize` repairs stay for the one thing the next section asks of them:
the value on screen is the value the file holds.

**Cases the spectrogram history goes without** (PR #1793), each with its cost when it occurs:

- A column dated wrongly, which no feeder writes: debug tests fail. In release one dated at or before the newest averages into each level's open slab; one dated far ahead erases every level's history and the heatmap draws blank until Clear display history, or until real time reaches that date.
- An analysis not given back before the next take, which the input worker's recovery after a panic rules out: a release build analyzes on from a fresh analyzer dated from the interrupted run's frame zero, so its columns date backward, as above.
- A crash inside the slab pyramid's fold: the levels it reached keep that column and the rest miss it, drawn so until the slabs scroll out; the input worker restarts its analysis as before.
- A Clear pressed while the input worker analyzes: the audio that step took from before the press stays in the emptied history, a few milliseconds normally, and as much as it took when it catches up after a stall (up to the 1.37 s audio ring).
- A project at 44.1 kHz, which is within the use: the live heatmap's fresh edge reads its lag at 48 kHz, 9% short (15 ms at Precise), inside the strip's 120 ms freshness allowance, so the cost is cheap. An export's edge is exact: it reads ahead at the soundtrack's own rate.
- The standalone synth below 5 frames a second: each frame restarts its run, so the heatmap and the curve draw nothing until frames come faster.

## Backwards compatibility is not a constraint

One or two personal Bitwig projects load this plugin, so a saved blob, a param range and a recorded automation lane are worth what it costs to reopen those projects and drag a bar back.
Narrow a range, rename a key, drop a field, move a default:
say what breaks in the PR body and make the change.
"This reinterprets saved state" is a line in the description, never a reason to keep a shape —
and never, on its own, a review finding.

What it does not license is a SILENT break.
The value on screen must still be the value the file holds, so a change of range or units carries whatever clamp or repair keeps the two agreeing (`ViewConfig::sanitize`, also applied at the scene entry).

The tree carries **no compat shims at all**, and that is the invariant to hold:
no `legacy_*` fields, migration passes or serde aliases for deleted variants.
A rename is a rename, a dropped variant is dropped.
At the editor's door a dropped enum variant costs a saved project the key that names it, which opens at its default with a Console line;
a recorded take whose look names one refuses to export, loudly.
Either is fine when the PR body says so.
Read the `persistence-contract` skill before changing a persisted shape.

## Fix what you find in this PR; an issue is for a substantial design decision

A defect, stale document or cleanup found along the way rides along in the PR at hand:
its own commit,
a test that fails first when the fix is code,
and its own line in the PR body so review reads it rather than skims past it.
Open an issue with `gh issue create` only for a substantial design decision that is Yan's to make —
a restructuring, a new mechanism, behaviour nobody asked to change —
carrying the evidence and the options.

A bug measured and not fixed still owes its record:
what it is NOT, by what measurement, what was tried and reverted, and what is left to try,
in the PR body beside the probes.
`BACKLOG.md` is no place for any of it:
it is gitignored and per-clone, so a worktree session has no copy of it.

## Claude permissions a worktree session needs go in `.claude/settings.json`

`.claude/settings.local.json` is gitignored, so a fresh worktree never gets a copy and its rules are inert exactly where most sessions run.
Rules that hold everywhere live in the checked-in `.claude/settings.json`;
per-machine paths and one-off grants stay local.
