---
name: capture-daw-state
description: Recover the plugin's live settings out of a Bitwig project — camera, layout, ViewConfig, params. Use when Yan has dialed in a look in the DAW and wants it captured as a new default, or when reproducing a bug against real saved state.
---

# Reading the plugin's live settings back out of Bitwig

Don't guess and don't read numbers off a screenshot —
the settings are spread across the Settings column's tabs (Tuning, Lattice, Analyzer, Mappings, Video, System), and bar positions don't give you floats.
The analyzer's own knobs are the Settings column's Analyzer tab, not the Analyzer section's tab of the same name, which is the picture.
The exact values are recoverable:

```sh
./read-plugin-state               # newest project: params, fields, editor settings
./read-plugin-state --rust        # view fields as an impl Default body
./read-plugin-state --appearance project.bwproject > appearance.ron
./read-plugin-state --raw         # what the file holds, before any restore
```

It is the `read-plugin-state` example in `crates/harmonigraph-plugin`, which reads each instance through the plugin's own restore (`harmonigraph_plugin::saved_state`),
so everything but `--raw` is what the plugin loads: values clamped, retired keys gone, the camera the host's.
The wrapper builds it in release first, a few minutes in a fresh worktree and seconds after, with the build's output on stderr.
`--raw` prints the saved params and fields as stored, `ui-state` decoded to its RON, for a persistence bug, where what the file holds is the question.

The UI state (`layout`, the editor preferences under `prefs` such as `folded_sections`, static camera geometry, ViewConfig) is saved as the editor shows it, window open or not (`UiState`, `crates/harmonigraph-plugin/src/editor/persist.rs`).
Camera movement belongs to the five `camera-*` host parameters;
the camera snapshot in `ui-state` can lag automation while the editor is closed, so every output but `--raw` lays the saved host parameters over it, as each editor frame does.
A project saved with the window open by a build from before #1301 holds the values of the last window close instead, and nothing warns;
if the numbers look stale, ask Yan to save it again from a current build.
Host-automatable params (the Lattice Map and its lanes, camera movement) were never affected,
which is why such a project can show fresh params next to a stale or missing `ui-state`.

The tuning axes, their links, Learn and the adaptive policy are not params (#1795):
the plugin saves them in its `musical-settings` field, the axes in microcents in `TuningAxis::index` order,
and the reader prints that field, as the plugin saves it after the restore, under `--- fields ---` with `editor-state` (the window), `lattice-maps` and `tuning-instance`.
A param the current build does not declare, such as a `tuning-*` one in a project saved before then, is listed as ignored on load, and `--raw` shows its value.

## Where the projects live

Not `~/Documents/Bitwig Studio/Projects` (that's empty) —
they're under `~/Library/CloudStorage/GoogleDrive-*/My Drive/music/`, which Spotlight does not index, so the reader walks that folder as well as asking `mdfind`.
Auto-backups count as saves.

A project using adaptive tuning holds more instances than the editor's:
each **Harmonigraph Tune** (its own plugin, `com.yan-h.harmonigraph-tune`) has no param and no `ui-state`, by design;
its Tuning delay is `delay` in its `tuning-instance` field, which the reader prints.
The reader prints every instance numbered and named, Harmonigraph or Harmonigraph Tune.
An output showing only Tune instances means the scan stopped short, not that the window was never closed.

## Scope check before you edit any default

- The fresh look is the `Default` of every group in `AppearanceDocument` (`crates/harmonigraph-ui/src/appearance.rs`):
`ViewConfig` (`crates/harmonigraph-scene/src/view/mod.rs`),
`SpectrumConfig` (`crates/harmonigraph-ui/src/config.rs`),
and the video block's `RenderConfig` and `RenderFrame` (`crates/harmonigraph-ui/src/video.rs`).
A capture edits each group's `impl Default`, not only `ViewConfig`'s —
`--rust` prints the view fields alone, so read the other groups off `--appearance`.
That `impl Default` is also every field's serde fallback (the `persistence-contract` skill), so there is no `default_*` block to keep in step.
- A saved blob MISSING a
key therefore picks the new value up, so "restyle the fresh view" and "restyle an under-specified saved view" are the same edit.
That is the intended trade, not an accident.
- Camera zoom, `layout` and `prefs.folded_sections` are navigation state, deliberately not baked into
defaults.
- A capture moves no golden frame:
all three sets draw from the pinned `crates/harmonigraph-golden/look.ron`, not from these defaults,
and `the_pinned_look_pins_every_field` keeps every saved field pinned there.
A golden that moves anyway is a fixture still reading a default, so pin that fixture rather than stating a picture change (the `pr-hygiene` skill).

## Using `--appearance`

The example's header documents the `.bwproject` container format, for when the scan itself needs fixing.
The version-7 editor save nests camera, view, spectrum and the whole video configuration under `appearance`;
the Spiral's framing is an editor preference under `prefs` beside it, so `--appearance` leaves it out.
`--appearance` writes that document, with the saved host camera parameters applied, for `harmonigraph-offline --appearance FILE`, and requires exactly one editor appearance in the project.
During replay, a take's recorded camera channels override the corresponding movement in a replacement appearance.
