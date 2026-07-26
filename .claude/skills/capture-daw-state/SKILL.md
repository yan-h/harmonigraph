---
name: capture-daw-state
description: Recover the plugin's live settings out of a Bitwig project — camera, dock, ViewConfig, params. Use when Yan has dialed in a look in the DAW and wants it captured as a new default, or when reproducing a bug against real saved state.
---

# Reading the plugin's live settings back out of Bitwig

When Yan has dialed in a look in the DAW and wants it captured (new
`ViewConfig::default()`, a bug reproduced against real state), don't guess
and don't read numbers off a screenshot — settings scroll across two tabs and
bar positions don't give you floats. The exact values are recoverable:

```sh
./read-plugin-state.py            # newest project: params, camera, view
./read-plugin-state.py --rust     # view fields as an impl Default body
```

**The trap, which costs a round trip with Yan every time it's missed:** the
UI state (dock, camera, ViewConfig) is written into the plugin state ONLY
when the editor WINDOW is closed (`impl Drop for LatticeEditorHandle`,
`crates/midi_lattice_3d/src/editor.rs`). Saving a project with the plugin
window open silently keeps the previous values, with no warning. So ask Yan
for, in order:

1. close the MIDI Lattice 3D **window**, then
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

- `ViewConfig::default()` is the fresh-view look — this is what you change.
- The `default_*` serde fns are what a blob PREDATING a field was drawn with.
  Don't touch those or you restyle already-saved views.
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
