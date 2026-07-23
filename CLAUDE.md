# CLAUDE.md

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
