---
name: capture-daw-state
description: Recover the plugin's live settings out of a Bitwig project — camera, layout, ViewConfig, params. Use when Yan has dialed in a look in the DAW and wants it captured as a new default, or when reproducing a bug against real saved state.
---

# Reading the plugin's live settings back out of Bitwig

When Yan has dialed in a look in the DAW and wants it captured (new fresh-look defaults, a bug reproduced against real state), don't guess and don't read numbers off a screenshot —
the settings are spread across the Settings column's tabs (Tuning, Lattice, Analyzer, Mappings, Video, System), and bar positions don't give you floats.
The analyzer's own knobs are the Settings column's Analyzer tab, not the Analyzer section's tab of the same name, which is the picture.
The exact values are recoverable:

```sh
./read-plugin-state.py            # newest project: params, camera, view
./read-plugin-state.py --rust     # view fields as an impl Default body
./read-plugin-state.py --appearance project.bwproject > appearance.ron
```

The UI state (`layout` and `folded_sections`, camera, ViewConfig) is saved as the editor shows it, window open or not (`UiState`, `crates/harmonigraph-plugin/src/editor/persist.rs`).
A project saved with the window open by a build from before #1301 holds the values of the last window close instead, and nothing warns;
if the numbers look stale, ask Yan to save it again from a current build.
Host-automatable params (tuning, fade, color range) were never affected —
they live in the param system, which is why such a project can show fresh params next to a stale or missing `ui-state`.

## Where the projects live

Not `~/Documents/Bitwig Studio/Projects` (that's empty) —
they're under `~/Library/CloudStorage/GoogleDrive-*/My Drive/music/`, which Spotlight does not index, so the script globs that folder as well as asking `mdfind`.
Auto-backups count as saves.

A project using adaptive tuning holds more instances than the editor's:
each **Harmonigraph Tune** (its own plugin, `com.yan-h.harmonigraph-tune`) has one `tuning_delay` param and no `ui-state`, by design.
The script prints every instance numbered, and labels a Tune as one;
the one with the `appearance` block is the editor's.
An output showing only Tune instances means the scan stopped short, not that the window was never closed.

## Scope check before you edit any default

- The fresh look is the `Default` of every group in `AppearanceDocument` (`crates/harmonigraph-ui/src/appearance.rs`):
`ViewConfig` (`crates/harmonigraph-scene/src/view/mod.rs`),
`SpectrumConfig` (`crates/harmonigraph-ui/src/config.rs`),
`SpiralView`,
and the video block's `RenderConfig` and `RenderFrame` (`crates/harmonigraph-take/src/render.rs`).
A capture edits each group's `impl Default`, not only `ViewConfig`'s —
`--rust` prints the view fields alone, so read the other groups off `--appearance`.
Each group carries a container-level `#[serde(default)]`, so its `impl Default` is also every field's serde fallback.
There is no second set of values to keep in step, and no `default_*` block to leave alone —
retuning the look here is free.
- What that costs is worth knowing before you retune: a saved blob MISSING a
key now picks the new value up.
That is the intended trade (backwards compatibility is not a constraint —
see CLAUDE.md), not an accident, but it means "restyle the fresh view" and "restyle an under-specified saved view" are the same edit.
- Camera zoom, `layout` and `folded_sections` are navigation state, deliberately not baked into
defaults.

## Container format, if the script ever needs fixing

`.bwproject` is a "BtWg" tagged binary.
Plugin state sits in a raw-DEFLATE section (wbits=-15, no zlib header) as nice-plug's plain JSON `{"version","params","fields"}`, and `fields["ui-state"]` is the RON from `SharedState::save_persist`.
The version-7 editor save nests camera, view, spectrum, spiral framing and the whole video configuration under `appearance`.
`--appearance` extracts that document for `harmonigraph-offline --appearance FILE`, requiring exactly one editor appearance in the project.
The take carries that appearance independently of the editor layout and its folds.
nice-plug can also zstd the JSON.
The script's own header documents this too.
