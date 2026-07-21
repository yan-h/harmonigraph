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

# Run the exact checks CI runs (clippy -D warnings + full test suite),
# locally. A green run here means a green run in GitHub Actions.
./ci.sh

# Build the CLAP/VST3 bundles (output in <target-dir>/bundled/).
cargo xtask bundle midi_lattice_3d --release

# Rebuild + hot-swap the binary into the main checkout's bundles (works
# from a git worktree; re-signs ad-hoc). Then rescan the plugin in the DAW.
./update-plugin.sh

# Read the plugin's live settings back out of a saved Bitwig project
# (--rust prints them as an impl Default body). CLOSE the plugin window
# and save the project first — the UI state is only written on window
# close. See the script header.
./read-plugin-state.py
```

Bundles land in `target/bundled/`.

To gate every `git push` on `./ci.sh` automatically, enable the tracked hook
once per clone: `git config core.hooksPath .githooks` (skip a one-off push
with `git push --no-verify`). This stands in for GitHub Actions when it's off.

> **Testing a branch in the DAW:** the DAW scans the **main checkout's**
> `target/bundled/`. `cargo xtask bundle` run from a worktree bundles the
> *main* sources (it walks to the topmost `Cargo.toml`), and each worktree
> has its own `target/`, so a branch build is otherwise invisible. Use
> `./update-plugin.sh` — it builds the current branch and swaps the fresh,
> re-signed binary into the bundles the DAW actually loads.

## Architecture

Dependencies point strictly downward; the fun layers never touch plugin
plumbing.

```
lattice-core    pure logic: PitchClass (integer microcents), Tuning,
                LatticePos, NoteTracker, SpectrumAnalyzer (FFT). No deps.
                Unit-tested. One module per concern; see its crate doc.
lattice-scene   per-frame view model: derive_scene() turns tracker+tuning
                into NodeInstances; orbit Camera; envelopes; CPU picking.
                Split style/view/camera/color/derive; see its crate doc.
lattice-render  wgpu renderer as an egui paint callback: instanced
                billboard nodes, WGSL in src/shaders/lattice.wgsl.
                *** Skins/effects/shaders iterate here. ***
lattice-ui      egui_dock pane shell: Lattice / Tuning / View / Appearance /
                Console / Spectral / Spectrum / Notes tabs, one file each
                under src/panes/. SharedState (incl. cross-pane hover),
                ParamBackend trait abstracting "where params live".
lattice-standalone  eframe dev harness: mock chord progression OR hardware
                    MIDI in (midir, with MPE bend decoding), plus a mock
                    synth feeding the spectrum analyzer.
midi_lattice_3d     nice-plug shell: params, MIDI + audio → two rtrb ring
                    buffers, custom wgpu egui editor (editor.rs) with
                    host-native window resizing, CLAP/VST3 exports.
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
  macOS/Windows in practice; the constant is the knob if a format mismatch
  ever panics on an exotic setup.
- **Depth sorting**: the lattice now renders through an offscreen
  color+depth pass, with bloom composited over it. The depth buffer is
  written but not yet read — nodes are still CPU-sorted back-to-front, so
  dense scenes can still show overlap artifacts. Enabling real depth
  testing is the remaining step (needs a two-pass opaque/transparent split;
  see [`docs/deferred-work.md`](docs/deferred-work.md)).
- **Skins**: the mechanism exists (`lattice_scene::skin`); add alternate
  skins, live re-skinning, and shader-side skin uniforms.
- **Spectral audio FFT**: done — the Spectral pane analyzes a real FFT of
  the plugin's audio input (mono mixdown of the input bus, gated on
  `show_audio`), no longer MIDI-only. Remaining work is analysis polish,
  not wiring.
- **Render styles**: the experimental set has been trimmed (NodeStyle 15→5,
  OuterStyle→4, CoreStyle folded into a solidity slider); a final aesthetic
  trim of the 5 surviving node styles is the remaining call. See
  [`docs/deferred-work.md`](docs/deferred-work.md).
