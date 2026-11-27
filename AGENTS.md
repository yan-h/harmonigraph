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
cargo build --release -p harmonigraph-plugin
```

The package builds the offline renderer too, as its `harmonigraph-offline` binary, so the video export is always the same commit as the plugin (#1954).
Skip the build only when nothing plugin-visible changed (docs, backlog, pure-test edits).

Push and open the draft PR first, then run `./session-lifecycle.sh handoff`, which runs that build while CI runs.
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
The pieces often carry tuning automation that moves the Lattice Map over time (Yan, 2026-11-14).
Yan improvises on a lattice-board, a Wicki-Hayden keyboard whose MIDI the project-wide Decode fifths reads, in meantone beside recorded 12-TET tracks,
and switches Note retuning to Lattice Map to compose in JI (Yan, 2026-11-20).
Yan no longer uses the Adaptive engine but keeps it in the product, so a case only Adaptive reaches lies outside the use (Yan, 2026-11-15),
except Adaptive with Decode fifths on, the lattice-board's strip, which is within it; Adaptive's search, with Decode fifths off, stays outside (#1896; Yan, 2026-11-23).
The projects run at 44.1 or 48 kHz (Yan, 2026-11-14).
Nothing in the rig sends All Notes Off or All Sound Off (CC 123 or 120), no project runs more than 16 Tunes, and Follow harmony goes unused but stays in the product, so a case only those reach lies outside the use (Yan, 2026-11-26, #1973 and #1977).
Yan never turns on the lattice's audio ring, so a case only it reaches lies outside the use,
and the pieces pan the lattice more than 12 fifths or 20 thirds within one Span (Yan, 2026-11-27, #1994).
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

- Saved editor state (the look, presets and undo) lost or changed with nothing saying so (#1740, PR #1764).
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

**Cases the tape goes without** (PR #1789), each with its cost when it occurs:

- More marks than one step holds (4,096): the rest are lost with a Console line, a lost release holds its note to the pass's end on playback, and a retrigger split at the cap can lose either half.
- A very dense pass: a chase replays it from its start, about 4.6 ms under the editor lock for 30 minutes of MPE pressure, once per cue or seam.
- A host block with no song position: a waiting cue still starts, the transport step reads the position before the gap, and play from there after an audition can print a false `LEAK?` line.
- A fresh cue: the lattice marks the last spot's map, or the live one, until its first block of audio arrives, a few tens of milliseconds.
- A song position past about 2.3 hours: zoomed fully out, the tape's heatmap draws more slabs than a live window holds.

Yan's answer to #1784 T3 cut these, so they are his decisions rather than gaps:

- A stalled input worker: a cue reads Playing until the next click, Stop or play.
- A whole-tape re-analysis: what is on screen fills in song order, about 15 s per 5 minutes of song at Balanced, up to about 40 s at Precise, and about a minute with the editor closed (measured in PR #1922).
- A cue at a speed other than 1× within about 180 ms after a gap: the audio before the gap may smear into its first milliseconds.
- A tempo change between passes: under a second of bar ticks past a short pass's end lands at its tempo.

**Cases the spectrogram history goes without** (PR #1793), each with its cost when it occurs:

- A column dated wrongly, which no feeder writes: debug tests fail. In release one dated at or before the newest averages into each level's open slab; one dated far ahead erases every level's history and the heatmap draws blank until Clear display history, or until real time reaches that date.
- A crash inside the slab pyramid's fold: the levels it reached keep that column and the rest miss it, drawn so until the slabs scroll out; the input worker restarts its analysis as before.
- A Clear pressed while the input worker analyzes: the audio that step took from before the press stays in the emptied history, a few milliseconds normally, and as much as it took when it catches up after a stall (up to the 1.37 s audio ring).
- A project at 44.1 kHz, which is within the use: the live heatmap's fresh edge reads its lag at 48 kHz, 9% short (15 ms at Precise), inside the strip's 120 ms freshness allowance, so the cost is cheap. An export's edge is exact: it reads ahead at the soundtrack's own rate.
- The standalone synth below 5 frames a second: each frame restarts its run, so the heatmap and the curve draw nothing until frames come faster.

**Cases the region folds go without** (PR #1785), each with its cost when it occurs:

- A project saved with an Analyzer region folded before region folds kept their space: it reopens with the Analyzer at its folded width, and unfolding gives the width back to nothing; a divider drag does.
- A section fold clicked on a frame whose pass egui repeats: it draws the new fold in the old window for that one frame.

**Cases the tuning settings go without** (PR #1795, and #1938 when the main thread took the record), each with its cost when it occurs:

- An edit still being written at a callback's start: it sounds one block later.
- A restore's maps, tuning and host parameters are three writes: a callback between them adopts one without the others for a block, recorded so in a take.
- A learned change reaches the sound and a take a worker step (4 ms with the window open, 20 ms shut) plus a block after its chord, where it was one block; in a faster-than-real-time bounce with Learn armed that is up to the bounce speed times 20 ms of song, and two runs can differ.
- A chord that starts and ends between two worker polls (4 ms with the window open, 20 ms shut): it can go unlearned (Yan, #1942).
- After lost input, editor edits and Learn keep applying until the host resets; the fault notice stays and a take recorded after the loss refuses its configuration (Yan's answer, #1795, recorded in `docs/intent/tuning.md`).

**Cases the glow goes without** (PR #1791), each with its cost when it occurs:

- More than `MAX_LIGHTS` (4,096) lit nodes reaching the lattice renderer, which no producer sends: `GlowFade` is the only cap, and the renderer checks it with a `debug_assert` alone. A release build grows the light strip to the next power of two and draws every halo up to the adapter's texture limit (16,384 rows on Apple silicon); past that, which only a window over 16,384 lit nodes could reach, creating the strip fails wgpu's validation.

**Cases the tuning Hub goes without** (PR #1801), each with its cost when it occurs:

- A row that fills to 64 voices after release copies were lost to a full 1,024-entry copy ring: Learn reads the stuck voices, as the lattice draws them, until the next cut; the dropped-event notice still shows.
- Past 2,048 records in one Hub callback: the same Tune's surplus waits each time rather than the rows taking turns.
- A second cut stacked on one whose Note-Offs the host has not yet taken, owing over 112 events: the surplus Note-Offs are dropped with no notice, a stuck note.
- A tuning expression past CLAP's ±120 semitones, which no host sends: a `debug_assert`; a release build stores a saturated pitch.
- A display lane overflow: no banner, a note that ended during the outage vanishes at the next snapshot without its fade, and an export draws a note held through the gap as one line; the export warning stays.
- A note a snapshot omitted and a later snapshot lists again, which no producer writes: it opens a second note rather than continuing the first.
- Lowering a Tune's delay while a take records: the notes after it are recorded at the row's latest time, up to the old delay late; a row's note times never go back (Yan, #1885).
- A miss from a full copy ring, read after a transport Stop: the Stop clears the ring's fault bit but keeps the count, so the Tune says "raise the delay", which is no remedy for a full ring.

**Cases drawing a missed note raw goes without** (#1799), each with its cost when it occurs:

- The usual miss, the Tune's whole callback run before the Hub's, reaches the Hub in the same pass as its onset: the note is drawn corrected only up to the row's latest scheduled time in that pass, nothing for a lone note and at most about the delay plus one buffer when the row had later events in that pass.
- A miss that reaches the Hub a pass after its onset, which only a race between the callbacks or a lost reply produces: the note is drawn corrected for up to about D plus one Hub callback.
- A missed note released in the same pass as its miss is read stays drawn corrected for its whole life.
- A miss record lost to a full copy ring leaves that note drawn corrected, with the dropped-event notice.
- The Hub's policy context keeps the pitch it decided for a missed note, which only Adaptive and Follow harmony read.
- Notes thrown away from a Tune's delay line at a cut (Stop, Reset, a reactivation) were already published, so the lattice and the take draw them from their onset until the cut's snapshot: up to about D of notes nobody heard.
- The host refusing an onset's tuning expression after accepting its note sends no miss, and the Tune's held voice keeps the correction, so the player's later per-note tuning jumps to the corrected pitch. Bitwig does not refuse output events.
- A miss the Hub reads a pass late, with a Tuning delay long enough that the withdrawal lands more than 50 ms after the onset: the ribbon moves to the raw pitch, while a name anchored at the onset keeps the corrected spelling for the note's life (`RollNote::settled_pitch` follows a bend only within `RollNote::SETTLE`), in the editor and in an export; a name a few cents off, or spelled the other way (Yan, #1804).

**Cases the lattice motion goes without** (PR #1882), each with its cost when it occurs:

- A Note fade dragged within about two Fades after a note shorter than its Fade: the note's lattice off moves with the bar, so dragged longer it can hold the note again, which arrives a second time; a glitch while dragging.
- A press Clear took while held, once its voice's fade ends: it leaves the replay, so a reset or a surface opened then (a Video preview) cuts the rest of its lattice departure.
- At the 4,096-note cap with no older pulse in `past`, `enforce_cap` evicts a pulse as it is filed, so a reset or a surface opened then loses the rest of its lattice departure; it takes about 4.5 notes a second sustained for 15 minutes to reach the cap.

**Cases the Console reveal goes without** (PR #1951; Yan, #1953), each with its cost when it occurs:

- A restore that reports a loss over a folded Settings section: Settings unfolds on the Console and the pictures squeeze until the next resize or fold click; the window does not grow.
- Two restores before an editor frame, which includes every restore while the editor is shut (a lossy project load, then a clean preset load or undo before opening it): the newer state opens with Settings unfolded on the Console, showing the older restore's line, and its next save stores that layout without marking the project.

**Cases the Tape harmony row goes without** (PR #1975; Yan, #1974), each with its cost when it occurs:

- A pitch off the live tuning, or outside the lattice's naming reach: it lights nothing in the row, as on the lattice. A recorded 12-TET track under meantone axes is mostly unlit; under just axes 8 of its 12 pitch classes land within the half-cent match of some node in the reach and light it, taking that node's name (F and G at 0.001¢; C♯, E, F♯, A♭ and B at about 0.28¢), and only 200, 300, 900 and 1000 cents stay unlit. (Yan, #1974; corrected 2026-11-27, PR #2000.)

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
