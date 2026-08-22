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
warns when the renderer it installs predates the branch's HEAD — the same
"matches the last commit" test the table applies to the dylib, and NOT a
comparison against the plugin beside it, which flags matched pairs as often as
mismatched ones — and prints the age of the one it is leaving in place when a
worktree built no renderer at all. But a warning during a load is a backstop
for a build that should have happened.

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
the tag takes one tick first: **Display tab → System page → Performance →
Performance overlay**. System is a PAGE behind the Display tab's picker, not a
tab of its own — a session that tells Yan to open a System pane is naming
something the dock does not have.
It opens in the editor's bottom-right corner and is DRAGGED from there, so
wherever it was last left is where it is — no session can say which corner to
look in.

A session handing over a build should say what the tag will read rather than say
"look at the overlay", because a HUD that says nothing new is exactly what a swap
that did not happen also looks like.

This exists because a swap can silently not have happened: no reactivate, a build
that landed in a different worktree, the wrong branch named, or a build that
never finished. Two builds are otherwise indistinguishable from inside the
DAW, and a look that is judged against the wrong binary costs a whole round
trip to discover.

**Sessions, when you hand over a build: say what tag it will show, and READ
it out of your dylib.** Not "loadable via `./load-plugin.sh <branch>`" alone —
name the tag too, so the first thing Yan can do is confirm the swap took. This
matters most when you hand over MORE THAN ONE build to compare (variants of a
look, an A/B of a fix): with several near-identical builds in play, "which one
am I looking at?" is the whole question, and the tag is the only answer that
cannot be fooled.

```
./load-plugin.sh --tag              # this worktree's build
./load-plugin.sh --tag <branch>     # some other worktree's
```

`./load-plugin.sh --list` prints the same thing for every worktree as its
`overlay` column, and a load prints the tag it just installed. Don't hand-roll
the `strings` pattern: the literals are laid out end to end in the binary, so
an unanchored match returns whatever was linked in front of the tag
(`avgseventh-node-occlusion @39a1325`). `--tag` anchors on the branch name.

**Do NOT derive the tag from a log.** Quoting `git log --oneline -1` is how a
handover names a commit the binary has never heard of, and it is wrong in the
ordinary case rather than the exotic one: the session order is edit → build →
commit → hand over, so the commit lands AFTER the build it is supposed to
describe and the binary carries its PARENT. Measured on a real handover, the
dylib was written at 20:06:46 and the commit it was reported as arrived at
20:07:42 — 56 seconds too late to be in it. An amend or a rebase breaks the
prediction the other way, leaving a stamped sha that is not an object on the
branch at all (`--list` says `gone from branch` for that one).

The tag names the COMMIT the build sat on, not the working tree — a build made
with uncommitted edits carries the commit under it. So commit BEFORE you build
if you want the tag to distinguish your work; if you build first, the tag is
still the truth about the binary, and it is the branch HEAD that is ahead.

## A reactivate only reloads if the sandbox process exits

Deactivate + reactivate re-reads the binary only when Bitwig's plug-in host
PROCESS for Harmonigraph exits on the unload. That is decided by **Settings →
Plug-ins → "Create a plug-in sandbox for:"**, and the default, `by Vendor`,
groups every plug-in sharing a `VENDOR` string into ONE process which lives as
long as ANY of them is loaded. A second plug-in of Yan's in the same project
therefore holds the old Harmonigraph image in memory and the reactivate is a
no-op — the DAW keeps drawing the previous build with nothing on screen saying
so, and only a full Bitwig restart clears it. `by Plug-in` and `Individually`
each give it a process of its own; `with Bitwig` loads it into the audio
engine, which unloads nothing.

The symptom is a HUD whose tag does not change after a load that reported
success. Check the hosting mode before suspecting the swap.

## Recovering a build someone else's swap evicted

Release builds land in `<that-worktree>/target/release/libharmonigraph_plugin.dylib`.
Match the dylib's mtime to the branch's last commit time to identify it, then
swap it back with `./load-plugin.sh <branch>` — which is the whole recovery,
and the only recipe here that gets the swap's ORDER right. To rebuild one
without cd'ing into the user's checkout:
`cargo build --release -p harmonigraph-plugin --manifest-path <main>/Cargo.toml`,
then load it the same way and verify via a distinctive string from that
branch's diff.

Do not hand-copy into the bundles and `codesign --force --sign -` each. That
order is the failure the next paragraph describes: signing the live bundle
discards the inode the copy just wrote, so a running host stays on the build
you were trying to replace while every command reports success.

Do NOT compare shasums against the source dylib to check a swap took:
`codesign --force` re-signs the bundled binary, so its hash legitimately
differs from the file just copied (the two bundle binaries match each OTHER).
Note that `codesign` also does not write through the file it signs — it
renames a new one into place, which is why `load-plugin.sh` signs a staging
copy and only then writes the finished bytes across. Confirm the new code is present instead, e.g.
`strings -a "<bundle>/Contents/MacOS/Harmonigraph" | grep -c "<new symbol>"`
— WGSL shader edits are embedded via `include_str!`, so a new const or
comment name greps cleanly.
