# Harmonigraph

A harmony visualizer that runs as an audio plugin (CLAP + VST3).

It draws what you play on a **Tonnetz**. That is a lattice with three
directions: a perfect fifth, a major third, and a harmonic seventh. Notes
close in harmony land close together, so a chord reads as a shape. Each axis
has its own tuning, so the lattice can be just, equal-tempered, or anywhere
between.

MIDI lights the lattice up. The audio input drives a spectrum analyzer and
spectrogram beside it. A piano roll keeps what was already played.

Successor to [midi_lattice](https://github.com/yan-h/midi_lattice).

Almost every line here was written by Claude Code sessions, directed and
reviewed by one human. [`CLAUDE.md`](CLAUDE.md) is the house style they work
under; `./ci.sh` is the gate every push clears.

Stack: Rust, [nice-plug](https://codeberg.org/RustAudio/nice-plug) (the
community continuation of nih-plug), egui 0.35, wgpu 29 (egui-baseview's
wgpu backend in the plugin, eframe's in the standalone harness).

## Setup

The Rust toolchain is pinned by `rust-toolchain.toml` (1.92). rustup
installs it on the first build.

**`sccache` has to be on `PATH`, or nothing builds.** `.cargo/config.toml`
sets `rustc-wrapper = "sccache"` for the whole workspace. Without it, every
cargo command here dies with `could not execute process sccache`.

```sh
brew install sccache
```

Why it is there: every worktree keeps its own `target/`, so parallel branches
never wait on a shared build lock. The cost is that each one would otherwise
recompile all ~465 dependencies from scratch. sccache serves those from a
single store, so only this repo's own crates recompile. To rule it out while
debugging a build failure, run `RUSTC_WRAPPER="" cargo build ...`.

`ffmpeg` is only for video export, and only if you want a playable file
rather than a frame sequence (`brew install ffmpeg`).

## Everyday commands

```sh
# The dev loop: the full UI + renderer in a plain window, with a mock chord
# progression or any connected MIDI port. No DAW needed.
cargo run -p harmonigraph-standalone

# The whole test suite; every crate carries tests.
cargo test

# The exact checks CI runs: clippy -D warnings, the tests, and the
# harmonigraph-core dependency guard. Green here means green in GitHub Actions.
./ci.sh

# Build the CLAP/VST3 bundles into target/bundled/.
cargo xtask bundle harmonigraph-plugin --release

# Read the plugin's live settings back out of a saved Bitwig project
# (--rust prints them as an impl Default body). CLOSE the plugin window
# and save the project first — the UI state is only written on window
# close. See the script header.
./read-plugin-state.py
```

To gate every `git push` on `./ci.sh` automatically, enable the tracked hook
once per clone: `git config core.hooksPath .githooks` (skip a one-off push
with `git push --no-verify`). This stands in for GitHub Actions when it's off.

## Getting a build into the DAW

The DAW scans exactly one place: the **main checkout's** `target/bundled/`.
Two things hide a branch build from it.

- Each worktree has its own `target/`. The DAW never looks there.
- `cargo xtask bundle` run from a worktree bundles the *main* sources, not
  the branch's. It walks up to the topmost `Cargo.toml`, which for a nested
  worktree is the main repo. The bundle looks fresh and holds none of your
  changes.

Two scripts sidestep both. Each copies the binary into the bundles the DAW
loads, then re-signs it ad-hoc — Apple Silicon requires that. Rescan or
restart the plugin afterwards.

```sh
# Build the current checkout — branch or main — and load it. One shot.
./update-plugin.sh

# Load a build that already exists, without building anything.
./load-plugin.sh              # menu of every worktree's build
./load-plugin.sh --list       # print the table, load nothing
./load-plugin.sh <branch>     # load that branch's build (substring ok)
```

The two differ in one way: `update-plugin.sh` builds, `load-plugin.sh` only
copies. That split exists because the bundle slot is shared. With several
branches in flight, each build stays in its own worktree. You then pick which
one goes live, rather than having them overwrite each other.

## Architecture

Dependencies point strictly downward; the fun layers never touch plugin
plumbing.

```
harmonigraph-core        pure logic, no dependencies at all. PitchClass
                         (integer microcents) and Tuning; lattice coordinates;
                         NoteTracker (what sounds now); NoteHistory/NoteRoll
                         (what was played, by pitch and by time); the FFT
                         spectrum analyzer and its spectrogram history; a
                         minimal WAV reader; audio<->MIDI onset alignment.
                         Unit-tested. One module per concern.
harmonigraph-scene       per-frame view model: derive_scene() turns
                         tracker+tuning into NodeInstances; orbit Camera;
                         envelopes; CPU picking. Split style/view/camera/
                         color/derive; see its crate doc.
harmonigraph-render      wgpu renderer as an egui paint callback: instanced
                         billboard nodes, WGSL in src/shaders/lattice.wgsl.
                         *** Skins/effects/shaders iterate here. ***
harmonigraph-ui          egui_dock pane shell: Lattice / Tuning / Nodes /
                         Scene / Console / Spectral / Analyzer / Notes /
                         Video / Panel tabs under src/panes/. SharedState
                         (the lattice's hovered node), ParamBackend trait
                         abstracting "where params live".
harmonigraph-take        the recorded input to a visualization: note events and
                         parameter automation on the audio clock. Linked into
                         the plugin, so serde+ron only. See
                         docs/offline-rendering.md.
harmonigraph-offline     offline video renderer: replays a take headless at an
                         exact frame rate, any resolution, own pane layout,
                         frames piped to ffmpeg. No window, no DAW, no realtime.
harmonigraph-standalone  eframe dev harness: mock chord progression OR hardware
                         MIDI in (midir, with MPE bend decoding), plus a mock
                         synth feeding the spectrum analyzer.
harmonigraph-plugin      nice-plug shell: params, MIDI + audio → two rtrb ring
                         buffers, custom wgpu egui editor (editor.rs) with
                         host-native window resizing, CLAP/VST3 exports.
```

Data flow in the plugin: the audio thread converts host MIDI to `NoteEvent`s
and pushes them into a lock-free ring buffer; the GUI thread drains it into
the `NoteTracker`, derives a `Scene`, and paints it. Parameters flow the
other way through `ParamBackend` (a `ParamSetter` in the plugin, plain
values in the harness), so every pane runs unmodified in both shells.

## Working on visuals

- `harmonigraph-render/src/shaders/lattice.wgsl` — node look, glow, animation.
- `harmonigraph-scene` — colors, envelopes, layout, camera behavior.
- Run the standalone harness; it uses the identical render path. No DAW
  needed until you're testing host integration.

## Version coupling

`egui-baseview 0.3` pins egui 0.35 / egui-wgpu 0.35 / wgpu 29 / baseview
0.1; `eframe` and `egui_dock` must match the egui version. All of this is
centralized in the workspace `Cargo.toml` — bump the whole cluster
together. `vendor/baseview` carries a small macOS fix (see PATCHES.md).

## What it does that v1 did not

v1 was a MIDI-only lattice. Everything below is new here, or rebuilt.

- **A sounding note is drawn in layers**, each dialed on its own. At the
  center is a core mark: a radius sizes it, a solidity slider morphs it from
  a soft glow to a solid orb, and one of four styles paints it (Steady,
  Vortex, Checker, Spiral). Around that, a radial band shows *which octaves*
  of the pitch class are sounding. Optional rings mark the highest and lowest
  held notes, so a chord's top and bottom line read at a glance.
- **Note names** with correct comma spelling, per-note tuning (MPE), tuning
  learned from a held chord, and v1's full channel semantics.
- **The Spectral pane** puts three things on one shared pitch axis: a real
  FFT of the audio input, the sounding voices, and a piano roll of what was
  played. The axis is continuous in cents, so bends and microtonal tunings
  sit between the keys instead of snapping to them. The pane also **turns to
  any of four sides**. The now-line — where the spectrum sits and a note
  arrives — can be its left, right, top or bottom. So the pane reads either
  as a tall strip beside the lattice or a wide one below it.
- **Host integration**: native window resizing, UI state that persists with
  the project, and single-gesture parameter automation.

## Known gaps / next steps

- **Surface format assumption**: the plugin editor assumes a `Bgra8Unorm`
  swapchain (see `ASSUMED_SURFACE_FORMAT` in `editor.rs`) because
  egui-baseview doesn't expose its wgpu `RenderState`. Correct on
  macOS/Windows in practice; the constant is the knob if a format mismatch
  ever panics on an exotic setup.
- **Depth sorting**: the lattice renders through an offscreen color+depth
  pass, with bloom composited over it. The depth buffer is written but not
  yet read — nodes are still CPU-sorted back-to-front, so dense scenes can
  still show overlap artifacts. Enabling real depth testing is the remaining
  step (needs a two-pass opaque/transparent split; see
  [`docs/deferred-work.md`](docs/deferred-work.md)).
- **Skins**: the mechanism exists (`harmonigraph_scene::skin`); add alternate
  skins, live re-skinning, and shader-side skin uniforms.
- **Making videos**: done, by offline replay rather than screen capture.
  Arm "Record take" in the Video pane and play the piece; the plugin writes
  a *take* (every note event and parameter change, stamped on the host
  transport so it lines up with a bounce). `harmonigraph-offline` replays it
  headless into an exact-CFR video at any resolution, with its own pane
  layout and the bounced audio muxed in. See
  [`docs/offline-rendering.md`](docs/offline-rendering.md). Live capture of
  the plugin window itself was investigated and rejected as the more
  expensive, lower-quality route —
  [`docs/window-recording.md`](docs/window-recording.md) has the analysis.
- **Spectral audio FFT**: done — the Spectral pane analyzes a real FFT of
  the plugin's audio input (mono mixdown of the input bus), no longer
  MIDI-only, and unconditionally rather than behind a setting. Remaining
  work is analysis polish, not wiring.
- **Render styles**: the experimental set has been trimmed (NodeStyle 15→4,
  OuterStyle removed outright, CoreStyle folded into a solidity slider); a
  final aesthetic trim of the 4 surviving node styles is the remaining call.
  See
  [`docs/deferred-work.md`](docs/deferred-work.md).

## License

Copyright (C) 2026 Yan Han.

Harmonigraph is free software: you can redistribute it and/or modify it
under the terms of the GNU General Public License as published by the Free
Software Foundation, either version 3 of the License, or (at your option)
any later version. See [`LICENSE`](LICENSE) for the full text.

This program is distributed in the hope that it will be useful, but WITHOUT
ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
more details.

### Exceptions

[`crates/harmonigraph-core`](crates/harmonigraph-core) is **`MIT OR Apache-2.0`**, not
GPL. It is the one general-purpose library here — dependency-free
just-intonation math, Tonnetz coordinates, and note spelling — and much of
that math descends from the permissively licensed
[midi_lattice v1](https://github.com/yan-h/midi_lattice), so it stays
permissive too. `ci.sh` enforces the property that justifies the split: the
crate must remain dependency-free. See
[its README](crates/harmonigraph-core/README.md).

The vendored forks under [`vendor/`](vendor) (`baseview`, `egui-baseview`)
are likewise **not** covered by the GPL — they remain under their upstream
`MIT OR Apache-2.0` terms, with their own license files in each directory.
See [`PATCHES.md`](PATCHES.md) for what was changed and why.

VST is a trademark of Steinberg Media Technologies GmbH.
