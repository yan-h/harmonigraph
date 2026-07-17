# MIDI Lattice 3D

A 3D Tonnetz MIDI visualizer plugin (CLAP + VST3): pitch classes on a
three-axis harmonic lattice (perfect fifths / major thirds / harmonic
sevenths, each with adjustable tuning), lit up by incoming MIDI notes with
per-octave indicators. Successor to [midi_lattice](https://github.com/yan-h/midi_lattice).

Stack: Rust, [nice-plug](https://codeberg.org/RustAudio/nice-plug) (the
community continuation of nih-plug), egui 0.35, wgpu 29 (egui-baseview's
wgpu backend in the plugin, eframe's in the standalone harness).

## Everyday commands

```sh
# The dev loop: full UI + renderer in a plain window with mock MIDI.
cargo run -p lattice-standalone

# Unit tests (core + scene logic).
cargo test

# Build the CLAP/VST3 bundles (output in <target-dir>/bundled/).
cargo xtask bundle midi_lattice_3d --release
```

Bundles land in `target/bundled/`.

## Architecture

Dependencies point strictly downward; the fun layers never touch plugin
plumbing.

```
lattice-core    pure logic: PitchClass (integer microcents), Tuning,
                LatticePos, NoteTracker. No deps. Unit-tested.
lattice-scene   per-frame view model: derive_scene() turns tracker+tuning
                into NodeInstances; orbit Camera; envelopes; CPU picking.
lattice-render  wgpu renderer as an egui paint callback: instanced
                billboard nodes, WGSL in src/shaders/lattice.wgsl.
                *** Skins/effects/shaders iterate here. ***
lattice-ui      egui_dock pane shell: Lattice / Tuning / Console / Spectral
                tabs, SharedState (incl. cross-pane hover), ParamBackend
                trait abstracting "where params live".
lattice-standalone  eframe dev harness with a mock chord progression.
midi_lattice_3d     nice-plug shell: params, MIDI → rtrb ring buffer,
                    custom wgpu egui editor (editor.rs) with host-native
                    window resizing, CLAP/VST3 exports.
```

Data flow in the plugin: audio thread converts host MIDI to `NoteEvent`s
and pushes them into a lock-free ring buffer; the GUI thread drains it into
the `NoteTracker`, derives a `Scene`, and paints it. Parameters flow the
other way through `ParamBackend` (a `ParamSetter` in the plugin, plain
values in the harness), so every pane runs unmodified in both shells.

## Working on visuals

- `lattice-render/src/shaders/lattice.wgsl` — node look, glow, animation.
- `lattice-scene` — colors, envelopes, layout, camera behavior.
- Run the standalone harness; it uses the identical render path. No DAW
  needed until you're testing host integration.

## Version coupling

`egui-baseview 0.3` pins egui 0.35 / egui-wgpu 0.35 / wgpu 29 / baseview
0.1; `eframe` and `egui_dock` must match the egui version. All of this is
centralized in the workspace `Cargo.toml` — bump the whole cluster
together. `vendor/baseview` carries a small macOS fix (see PATCHES.md).

## Features beyond v1

- Octave indicators (six switchable styles), note-name labels with comma
  spelling, a spectral pitch-class meter with two-way hover sync, tuning
  learn from held chords, per-note tuning (MPE), v1's full channel
  semantics, host-native window resizing, UI state persistence, and
  single-gesture parameter automation.

## Known gaps / next steps

- **Surface format assumption**: the plugin editor assumes a `Bgra8Unorm`
  swapchain (see `ASSUMED_SURFACE_FORMAT` in `editor.rs`) because
  egui-baseview doesn't expose its wgpu `RenderState`. Correct on
  macOS/Windows in practice; worth upstreaming a fix.
- **Depth/bloom**: the lattice draws into egui's pass (no depth buffer;
  CPU-sorted back-to-front). For dense scenes or post-processing, render
  to an offscreen texture in `prepare()` instead.
- **Clock reconciliation**: GUI re-stamps note events on arrival; fine
  visually, but sub-frame event spacing is lost.
- **Skins**: the mechanism exists (`lattice_scene::skin`); add alternate
  skins, live re-skinning, and shader-side skin uniforms.
- **Spectral audio FFT**: the pane is MIDI-derived today.
- **Render styles**: Node style / Chord edges are switchable experiments;
  prune the losers after evaluation.
- **Upstreaming**: prepared fixes in `docs/upstream/`.
