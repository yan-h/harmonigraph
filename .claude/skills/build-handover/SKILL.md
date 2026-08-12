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
loadable. After a swap, deactivate + reactivate the plugin in Bitwig to pick
it up — a rescan does not reload one that is already loaded. Deactivate it
BEFORE the swap when a session is worth protecting: the copy rewrites code
the running host has not faulted in yet.

- Both `load-plugin.sh` and `update-plugin.sh` record the live build in
  `target/bundled/.loaded`, so "what's loaded?" is answerable without guessing.
- `./update-plugin.sh` remains the build-and-load-in-one-shot path (it builds
  the checkout it runs FROM and swaps that immediately) — use it when you
  explicitly want a session to make its own build live, e.g. a single-session
  flow. Run it from the main checkout and it rebuilds main, not your branch.

## The renderer is a SECOND slot, and it goes stale on its own

Video export does not run in the plugin. The Render pane spawns
`~/Library/Application Support/Harmonigraph/harmonigraph-offline`
(`harmonigraph-record`'s default path), and `load-plugin.sh` installs that
binary from the same worktree it takes the dylib from — so a load is really
two swaps, and only the first one is guaranteed to be current.

The renderer draws through `harmonigraph-ui` and `harmonigraph-render` exactly
as the editor does, so it goes out of date on any change to the picture, not
only on changes under `crates/harmonigraph-offline/`. But nothing rebuilds it
unless a session names `-p harmonigraph-offline`, and `load-plugin.sh` copies
whatever is in `target/release` without minding its age. A session that builds
only the plugin therefore hands over a matched editor and a renderer from some
earlier commit, and the pair is indistinguishable from a matched one until an
export comes back drawn the old way. PR #340's lead is the worked example: it
was live in the editor within minutes and absent from every mp4 for hours.

Which is why the build line in `CLAUDE.md` names both packages. `load-plugin.sh`
warns when the renderer it installs is much older than the plugin beside it,
and prints the age of the one it is leaving in place when a worktree built no
renderer at all — but a warning during a load is a backstop for a build that
should have happened.

To check the live pair directly, without a render:

```
ls -la ~/Library/Application\ Support/Harmonigraph/harmonigraph-offline
strings -a ~/Library/Application\ Support/Harmonigraph/harmonigraph-offline | grep -c "<new symbol>"
```

A persisted config key is the reliable symbol — the serde field names are in
the binary, so `roll_lead` answers "does this renderer know about the lead?"
the same way a WGSL const answers it for the plugin.

## Every build says which build it is

The performance overlay's bottom line reads `build  <branch> @<sha>` — the
branch with its `worktree-` prefix stripped, so it is exactly the argument
`./load-plugin.sh <branch>` takes. It is stamped at compile time by
`crates/harmonigraph-perf/build.rs`. The overlay carrying it ships OFF, so reading
the tag takes one tick first: **System pane → Performance → Performance overlay**.
Then it is in the corner of the Analyzer pane — or of the Lattice, which is where
the overlay goes when the Analyzer is folded or off screen.

A session handing over a build should say so rather than say "look at the
corner", because an empty corner is exactly what a swap that did not happen also
looks like.

This exists because a swap can silently not have happened: no reactivate, a build
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

Release builds land in `<that-worktree>/target/release/libharmonigraph_plugin.dylib`.
Match the dylib's mtime to the branch's last commit time to identify it, then
swap it back. To rebuild one without cd'ing into the user's checkout:
`cargo build --release -p harmonigraph-plugin --manifest-path <main>/Cargo.toml`,
then hand-copy the dylib into both bundles and `codesign --force --sign -`
each, and verify via a distinctive string from that branch's diff.

Do NOT compare shasums against the source dylib to check a swap took:
`codesign --force` rewrites the bundled binary's signature in place, so its
hash legitimately differs from the file just copied (the two bundle binaries
match each OTHER). Confirm the new code is present instead, e.g.
`strings -a "<bundle>/Contents/MacOS/Harmonigraph" | grep -c "<new symbol>"`
— WGSL shader edits are embedded via `include_str!`, so a new const or
comment name greps cleanly.
