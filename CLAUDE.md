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
