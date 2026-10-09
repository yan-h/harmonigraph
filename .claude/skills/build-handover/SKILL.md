---
name: build-handover
description: How to load a branch's build into the DAW, how the build tag identifies it, and why sessions push before building and never use cargo xtask bundle. Use when handing over a build for Yan to look at, comparing more than one build, or when a swap seems not to have taken.
---

# Loading a build, and knowing which build you loaded

## Why the slot is pulled, not pushed

Bitwig loads exactly ONE plugin build:
the main checkout's `target/bundled/Harmonigraph.clap`.
A branch or worktree build is invisible in the DAW until its binary is swapped into that slot.
With parallel sessions that slot is shared, so every session builds into its own worktree and Yan chooses which build goes live;
a session touching the slot itself would evict whatever he is currently testing.

**Push before you build, not after.** CI reads the pushed commit and never reads `target/`, so the release build and the checks can overlap.
Building first serializes them for no reason:
a 1m28s build followed by a 6-7 minute CI run is eight minutes where pushing first is seven.

**Don't use `cargo xtask bundle` from a nested Claude worktree** —
nice-plug-xtask's `chdir_workspace_root()` takes the *topmost* ancestor with a `Cargo.toml` (`ancestors().filter(has Cargo.toml).last()`), which for a nested worktree is the main repo root, so it silently builds main.
The bundle looks fresh and contains none of the branch's changes.
`load-plugin.sh` and `update-plugin.sh` exist to sidestep this.

## Yan: load whichever build you want

- `./load-plugin.sh` — menu of every worktree's build (freshness + which one
is live now);
pick a number to swap it in.
- `./load-plugin.sh <branch>` — load that branch's build directly (unique
substring is fine).
- `./load-plugin.sh --list` — just print the table, load nothing.

It copies only, never builds;
a build must already exist in a registered worktree or a verified lifecycle handoff, which holds both the plugin and the offline renderer and stays loadable after source cleanup.
Stale builds (dylib older than the branch's HEAD) are flagged but still loadable.
After installing, the loader reports which Bitwig plug-in hosts and audio engine still map the old or the new executable;
that is one process snapshot, not a check of the running build tag.
A failed diagnostic never fails an otherwise successful install.

The loader discovers registered Git worktrees, so a Codex-managed worktree appears on the same menu without configuration.
It does need a branch:
a detached build appears as `(detached)`, cannot be selected uniquely by branch, and cannot carry a useful overlay tag.

- Both `load-plugin.sh` and `update-plugin.sh` record the live build in
`target/bundled/.loaded`, so "what's loaded?" is answerable without guessing.
- `./update-plugin.sh` remains the build-and-load-in-one-shot path (it builds
the checkout it runs FROM and swaps that immediately) —
use it when you explicitly want a session to make its own build live, e.g. a single-session flow.
Run it from the main checkout and it rebuilds main, not your branch.

## The renderer is a SECOND slot, and it goes stale on its own

Video export does not run in the plugin.
The Video pane spawns `~/Library/Application Support/Harmonigraph/harmonigraph-offline` (`harmonigraph-record`'s default path), and `load-plugin.sh` installs that binary from the same worktree it takes the dylib from —
so a load is really two swaps, and only the first one is guaranteed to be current.

Nothing rebuilds the renderer unless a session names `-p harmonigraph-offline`, and `load-plugin.sh` copies whatever is in `target/release` without minding its age, so a plugin-only build hands over a renderer from some earlier commit (`CLAUDE.md` has the rule and PR #340).
Naming both is not a second full build either:
the two share every dependency and the whole UI, so the renderer costs a link on top of a plugin build that is already done.
`load-plugin.sh` warns when the renderer it installs predates the branch's HEAD —
the same "matches the last commit" test the table applies to the dylib, and NOT a comparison against the plugin beside it, which flags matched pairs as often as mismatched ones —
and prints the age of the one it is leaving in place when a worktree built no renderer at all.
That warning is a backstop for a build that should have happened.

To check the live pair directly, without a render:

```
ls -la ~/Library/Application\ Support/Harmonigraph/harmonigraph-offline
strings -a ~/Library/Application\ Support/Harmonigraph/harmonigraph-offline | grep -c "<new symbol>"
```

A persisted config key is the reliable symbol —
the serde field names are in the binary, so `roll_lead` answers "does this renderer know about the lead?" the same way a WGSL const answers it for the plugin.

## Every build says which build it is

The performance overlay's bottom line reads `build  <branch> @<sha>` —
the branch as git names it, `claude/<name>` or `codex/<slug>` (only a legacy `worktree-` prefix is stripped).
The result is exactly the argument `./load-plugin.sh <branch>` takes.
It is stamped at compile time by `build/build_tag.rs`, which each leaf binary's own `build.rs` includes (plugin, offline, standalone).
The overlay carrying it ships OFF, so reading the tag takes one tick first:
**System tab → Performance → Performance overlay**.
System is a tab of the Settings column, and may sit in that strip's overflow menu when the column is narrow.
It opens in the editor's bottom-left corner and is DRAGGED from there, so no session can say which corner to look in.

A swap can silently not have happened:
a surviving host process, a build that landed in a different worktree, the wrong branch named, or a build that never finished.
Two builds are otherwise indistinguishable from inside the DAW, and a HUD that says nothing new is exactly what a failed swap looks like.

**Sessions, when you hand over a build:
say what tag it will show, and READ it out of your dylib**, so the first thing Yan can do is confirm the swap took.
This matters most when you hand over MORE THAN ONE build to compare (variants of a look, an A/B of a fix):
with several near-identical builds in play, "which one am I looking at?" is the whole question, and the tag is the only answer that cannot be fooled.

```
./load-plugin.sh --tag              # this worktree's build
./load-plugin.sh --tag <branch>     # some other worktree's
```

`./load-plugin.sh --list` prints the same thing for every worktree as its `overlay` column, and a load prints the tag it just installed.
Don't hand-roll the `strings` pattern:
the literals are laid out end to end in the binary, so an unanchored match returns whatever was linked in front of the tag (`avgseventh-node-occlusion @39a1325`).
`--tag` anchors on the branch name.

**Do NOT derive the tag from a log.** The tag names the COMMIT the build sat on, not the working tree, so a build made before the commit it is reported as carries that commit's PARENT, and quoting `git log --oneline -1` names a commit the binary has never heard of.
Measured on a real handover, the dylib was written at 20:06:46 and the commit it was reported as arrived at 20:07:42 —
56 seconds too late to be in it.
An amend or a rebase breaks the prediction the other way, leaving a stamped sha that is not an object on the branch at all (`--list` says `gone from branch` for that one).
So commit BEFORE you build if you want the tag to distinguish your work.

## Reload a build through the audio engine

The installed executable is re-read when Bitwig replaces the process that mapped the old one.
After the loader finishes,
deactivate and reactivate Bitwig's audio engine;
that is the supported reload gesture in the tested setup.
An individual device toggle, editor reopen or rescan alone may leave that process alive.

Individual device deactivation is a narrower lifecycle.
Its behaviour depends on **Settings → Plug-ins → "Create a plug-in sandbox for:"**:
the default,
`by Vendor`,
groups every plug-in sharing a `VENDOR` string into one process that can live as long as any of them is loaded.
A second plug-in of Yan's in the same project can therefore keep the old Harmonigraph image mapped across one device's toggle.
`by Plug-in` and `Individually` each give it a process of its own;
`with Bitwig` maps it in the audio engine itself,
making the engine cycle the relevant lifecycle boundary.

The symptom of a surviving process is a HUD whose tag does not change after a successful installation.
If that happens,
read the loader's process diagnostic and fall back to a full Bitwig restart.
The loader intentionally preserves that process's old file instead of changing mapped executable bytes underneath it.

## Recovering a build someone else's swap evicted

Release builds land in `<that-worktree>/target/release/libharmonigraph_plugin.dylib`.
Completed `session-lifecycle` handoffs are preserved under the common Git directory's `agent-lifecycle/builds/`, and `./load-plugin.sh <branch>` finds the latest one when no local binary remains;
reading them needs agent-config's `session-lifecycle` skill installed.
Match the dylib's mtime to the branch's last commit time to identify it, then swap it back with `./load-plugin.sh <branch>` —
which is the whole recovery, and the only recipe here that gets the swap's ORDER right.
To rebuild one without cd'ing into the branch's own worktree:
`cargo build --release -p harmonigraph-plugin -p harmonigraph-offline --manifest-path <that-worktree>/Cargo.toml`, then load it the same way and verify via a distinctive string from that branch's diff.

Use the loader rather than overwriting an installed executable by hand.
Two consecutive ordinary installs produced macOS `CODESIGNING Invalid Page` scanner kills while the on-disk signature still verified ([#705](https://github.com/yan-h/harmonigraph/issues/705)).
The loader instead signs a staging bundle and renames a fresh sibling executable atomically into place, so the executable inode changes on purpose.

Do NOT compare shasums against the source dylib to check a swap took:
`codesign --force` re-signs the bundled binary, so its hash legitimately differs from the file just copied (the two bundle binaries match each OTHER).
Confirm the new code is present instead, e.g. `strings -a "<bundle>/Contents/MacOS/Harmonigraph" | grep -c "<new symbol>"` —
WGSL shader edits are embedded via `include_str!`, so a new const or comment name greps cleanly.
