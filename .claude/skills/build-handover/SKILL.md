---
name: build-handover
description: How to load a branch's build into the DAW and how the build tag identifies it. Use when handing over a build for Yan to look at, comparing more than one build, or when a swap seems not to have taken.
---

# Loading a build, and knowing which build you loaded

The always-loaded contract is in `CLAUDE.md`: sessions build before they
pause and do NOT swap the shared slot. This file is the mechanics.

## Yan: load whichever build you want

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

## Every build says which build it is

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

## Recovering a build someone else's swap evicted

Release builds land in `<that-worktree>/target/release/libmidi_lattice_3d.dylib`.
Match the dylib's mtime to the branch's last commit time to identify it, then
swap it back. To rebuild one without cd'ing into the user's checkout:
`cargo build --release -p midi_lattice_3d --manifest-path <main>/Cargo.toml`,
then hand-copy the dylib into both bundles and `codesign --force --sign -`
each, and verify via a distinctive string from that branch's diff.

Do NOT compare shasums against the source dylib to check a swap took:
`codesign --force` rewrites the bundled binary's signature in place, so its
hash legitimately differs from the file just copied (the two bundle binaries
match each OTHER). Confirm the new code is present instead, e.g.
`strings -a "<bundle>/Contents/MacOS/MIDI Lattice 3D" | grep -c "<new symbol>"`
— WGSL shader edits are embedded via `include_str!`, so a new const or
comment name greps cleanly.
