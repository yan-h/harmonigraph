---
name: capture-daw-state
description: Recover the plugin's live settings out of a Bitwig project — camera, dock, ViewConfig, params. Use when Yan has dialed in a look in the DAW and wants it captured as a new default, or when reproducing a bug against real saved state.
---

# Reading the plugin's live settings back out of Bitwig

When Yan has dialed in a look in the DAW and wants it captured (new
`ViewConfig::default()`, a bug reproduced against real state), don't guess
and don't read numbers off a screenshot — the settings are spread across three
tabs (Tuning, Display, Video), one of which folds four more pages inside it
(Colors, Lattice, Analyzer, System, behind a picker row), and bar positions
don't give you floats. The analyzer's own knobs are the Display tab's Analyzer
page, not the Analyzer tab, which is a picture. The exact values are
recoverable:

```sh
./read-plugin-state.py            # newest project: params, camera, view
./read-plugin-state.py --rust     # view fields as an impl Default body
```

**The trap, which costs a round trip with Yan every time it's missed:** the
UI state (dock, camera, ViewConfig) is written into the plugin state ONLY
when the editor WINDOW is closed (`impl Drop for LatticeEditorHandle`,
`crates/harmonigraph-plugin/src/editor.rs`). Saving a project with the plugin
window open silently keeps the previous values, with no warning. So ask Yan
for, in order:

1. close the Harmonigraph **window**, then
2. save the project (Cmd+S).

Only then run the script. Asking for a save alone gets you nothing.
Host-automatable params (tuning, fade, color range) are exempt — they live in
the param system and are always current, which is why a project can show
fresh params next to a missing `ui-state`. That mismatch is the tell.

## Where the projects live

Not `~/Documents/Bitwig Studio/Projects` (that's empty) — they're under
`~/Library/CloudStorage/GoogleDrive-*/My Drive/music/`, and the script finds
them via `mdfind`. Auto-backups count as saves.

## Scope check before you edit any default

- `ViewConfig::default()` is the fresh-view look, and since PR #251 it is the
  ONLY place to change: the struct carries a container-level
  `#[serde(default)]`, so `impl Default` is also every field's serde fallback.
  There is no second set of values to keep in step, and no `default_*` block
  to leave alone — retuning the look here is free.
- What that costs is worth knowing before you retune: a saved blob MISSING a
  key now picks the new value up. That is the intended trade (backwards
  compatibility is not a constraint — see CLAUDE.md), not an accident, but it
  means "restyle the fresh view" and "restyle an under-specified saved view"
  are the same edit.
- Camera zoom and dock are navigation state, deliberately not baked into
  defaults.

## Container format, if the script ever needs fixing

`.bwproject` is a "BtWg" tagged binary. Plugin state sits in a raw-DEFLATE
section (wbits=-15, no zlib header) as nice-plug's plain JSON
`{"version","params","fields"}`, and `fields["ui-state"]` is the RON from
`SharedState::save_persist`. nice-plug can also zstd the JSON. The script's
own header documents this too.

## Known follow-up, not yet asked for

Writing the blob only on window close also means a Bitwig crash loses view
settings. A periodic flush would fix it.
