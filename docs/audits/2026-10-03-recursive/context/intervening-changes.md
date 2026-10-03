# Intervening source changes

Baseline: ad1c6e6b9474dd80098703ae15cac241bba0eb41.
First remote refresh: 47763080, appearance keyboard undo (#1433).
A separate investigator inspected the exact seven-file diff against baseline.

Unchanged: renderer/scene/shaders/resources, tuning drain, UiState serialization/restore and lock order, 20 ms fallback, camera parameter ownership, look capture exclusions and per-A/B history storage.

Changed: focused plugin now reserves Cmd/Ctrl-Z appearance shortcuts, including empty appearance history.
Keyboard availability of host camera undo while plugin-focused therefore changes although ownership does not.
History processing remains after body edits and before camera gesture completion.
New unit tests cover body edits, text undo, Undo+Enter focus release, shifted Z and modifiers; native Bitwig routing remains unverified.

Follow-up: at a safe host session test focused appearance undo/redo, text undo, unfocused host undo and ordinary DAW shortcuts.
Use explicit host undo or move focus when validating camera history; do not assume focused Cmd-Z still reaches Bitwig.
Pinned audit conclusions do not require a new broad scan.

## Second remote refresh: 77c453a42 (#1431)

The exact `47763080..77c453a42` diff changes piano-roll density layout and paint ordering (`ui/panes/spectral/roll.rs`, new `roll/density.rs`), adds a `RollInstance` tremolo attribute and roll-shader RGB hatching (`render/roll.rs`, `shaders/roll.wgsl`), regenerates the corresponding Metal corpus, and adds geometry, shader-pixel and six offline-pipeline density fixtures plus rendered evidence and updated roll intent.
Readable repeats now reserve visible gaps; unresolved repeats join one textured ribbon while factual note times, tuned onset pitch, intensity and interleaved-voice paint order remain represented.
The new offline fixture reaches 4/20/80-second spans at 1×/2× density, rather than merely constructing a short repeated-note run.

Revalidate any audit claim about piano-roll readability, roll instance count, roll CPU preparation, roll GPU draw cost or whole-export timing with a roll visible, especially at dense-repetition zoom.
The new path groups notes, allocates ribbon/paint vectors, splits draw spans and sorts them by original onset order; no timing in this pinned audit measures that current-main cost.
The previously collected baseline timings remain evidence only for their pinned source and workload, not measurements of `77c453a42`.
Temporal/event conclusions remain scoped: replayed roll records keep factual times, but current-main roll pixels and any cross-cadence image comparison need the new roll path if asserted.

No source in this diff changes recorder/replay publication, editor save/restore, scene normalization, hidden-surface ownership, spectral analyzer/color memory, sheet binding or atmosphere target keys.
Those pinned conclusions do not need a broad repeat solely because of #1431.
The refreshed main has not been merged or rebased into the audit worktree.
