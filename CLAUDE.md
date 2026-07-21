# CLAUDE.md

## Pausing = loaded in Bitwig (swap the bundle first)

Bitwig loads exactly ONE plugin build: the main checkout's
`target/bundled/MIDI Lattice 3D.{clap,vst3}`. A branch or worktree build is
invisible in the DAW until its binary is swapped into that slot.

Yan assumes that whenever a session pauses, its changes are already loaded
and testable in Bitwig. So before ending ANY turn after changing
plugin-affecting code — task complete, blocked on a question, partial
progress — run `./update-plugin.sh` from YOUR checkout and end your message
by stating which branch is now loaded.

- The script builds whatever checkout it runs FROM. Run it from your
  worktree; run from the main checkout it silently rebuilds main and
  reinstalls that (this exact miss has burned real sessions).
- Skip the swap only when nothing plugin-visible changed (docs, backlog,
  pure test changes) — swapping then would just evict the build Yan is
  currently testing.
- The bundle is a single slot and the last pauser wins; that's accepted.
  Always announce what you loaded so the current state is known. Evicted
  builds survive in each worktree's own `target/release/` and can be
  re-swapped the same way.
- After a swap, the plugin must be reopened/rescanned in Bitwig to load the
  new binary; say so.
- Don't use `cargo xtask bundle` from a nested worktree — it resolves the
  topmost workspace and builds main. `update-plugin.sh` exists to sidestep
  this; see its header comment.

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
