# Harmonigraph

![The pitch lattice on the left, the spectrum analyzer and its MIDI-overlaid spectrogram in the middle, the Display settings on the right, mid-passage](docs/images/plugin-window-lattice-analyzer-display.png)

A harmony visualizer that runs as an audio plugin (CLAP + VST3).
I use it for checking my tuning when composing in my DAW, and for generating visualizations of my pieces.

The plugin draws incoming MIDI on a 3-dimensional [Tonnetz](https://en.wikipedia.org/wiki/Tonnetz).

- That is, a lattice with three directions: a perfect fifth, a major third, and a harmonic seventh.
- Each lattice node represents a pitch class. Pitch height of individual voices is represented by circular "slices".
- Each axis has its own tuning, so the lattice can be just, equal-tempered, or anywhere between.

It also includes a spectrum analyzer for incoming audio, which feeds a spectrogram overlayed with incoming MIDI.
The Analyzer page can read either the main input or a host-routed sidechain;
Main is the default, and an unrouted Sidechain is silence rather than an automatic fallback.
The same choice drives the spectrum, spectrogram, Spiral, lattice audio rings, and the audio recorded with a video take.

See the [settings guide](docs/settings.md) for the control layout and units.

Demonstration:
[slipstream (5-limit just intonation)](https://www.youtube.com/watch?v=VuD9JOmi6_o).

Successor to [midi_lattice](https://github.com/yan-h/midi_lattice).

## Planned adaptive tuning

Project-wide adaptive MIDI retuning is designed but not implemented.
The plan adds one lightweight Harmonigraph Tune note effect before each participating instrument path;
one full Harmonigraph sequences new attacks across tracks and returns their tuning assignments for output after a fixed delay.
Each assignment takes the preceding ones into account, and its adaptive correction stays fixed through release while composing with later player pitch expression.
A missed assignment deadline delays the note further and reports a failure, rather than dropping it or emitting an unretuned attack.
See [`docs/adaptive-tuning.md`](docs/adaptive-tuning.md) for the decided behavior, the real-time protocol and the deliberately deferred alternatives.

Almost every line here was written by Claude Code sessions, directed and reviewed by one human.
[`CLAUDE.md`](CLAUDE.md) is the house style they work under;
GitHub Actions runs `./ci.sh` as the canonical full gate.

Stack:
Rust, [nice-plug](https://codeberg.org/RustAudio/nice-plug) (the community continuation of nih-plug), egui 0.35, wgpu 29 (egui-baseview's wgpu backend in the plugin, eframe's in the standalone harness).

**Tested only on macOS, in Bitwig Studio.** No other OS or host has been tried.
Several [`vendor/`](vendor) patches are macOS-only too —
`cfg`-gated, so elsewhere it still builds, just without those fixes ([`PATCHES.md`](PATCHES.md)).

## Setup

The Rust toolchain is pinned by `rust-toolchain.toml` (1.92).
rustup installs it on the first build.

**`sccache` has to be on `PATH`, or nothing builds.** `.cargo/config.toml` sets `rustc-wrapper = "sccache"` for the whole workspace.
Without it, every cargo command here dies with `could not execute process sccache`.

```sh
brew install sccache
```

Why it is there:
every worktree keeps its own `target/`, so parallel branches never wait on a shared build lock.
The cost is that each one would otherwise recompile all ~465 dependencies from scratch.
sccache serves those from a single store, so only this repo's own crates recompile.
To rule it out while debugging a build failure, run `RUSTC_WRAPPER="" cargo build ...`.

`ffmpeg` is only for video export, and only if you want a playable file rather than a frame sequence (`brew install ffmpeg`).

## Everyday commands

```sh
# The dev loop: the full UI + renderer in a plain window, with a mock chord
# progression or any connected MIDI port. No DAW needed.
cargo run -p harmonigraph-standalone

# The whole test suite; every crate carries tests.
cargo test

# The canonical full gate used by GitHub Actions: formatting, workspace
# clippy and tests, the plugin package check, harmonigraph-render's own tests,
# both vendored crates, doc links, the harmonigraph-core dependency guard,
# the worktree-reclaim lock cases and the bundle swap.
./ci.sh

# Build the CLAP/VST3 bundles into target/bundled/.
cargo xtask bundle harmonigraph-plugin --release

# Read the plugin's live settings back out of a saved Bitwig project
# (--rust prints them as an impl Default body). CLOSE the plugin window
# and save the project first — the UI state is only written on window
# close. See the script header.
./read-plugin-state.py
```

Enable the tracked pre-push formatting check once per clone with `git config core.hooksPath .githooks`.
The hook stays cheap locally;
GitHub Actions runs the full `./ci.sh` gate for pull requests and pushes to `main`.

## Getting a build into the DAW

The DAW scans exactly one place:
the **main checkout's** `target/bundled/`.
Two things hide a branch build from it.

- Each worktree has its own `target/`. The DAW never looks there.
- `cargo xtask bundle` run from a worktree bundles the *main* sources, not the branch's. It walks up to the topmost `Cargo.toml`, which for a nested worktree is the main repo. The bundle looks fresh and holds none of your changes.

Two scripts sidestep both.
Each copies the binary into the bundles the DAW loads, then re-signs it ad-hoc —
Apple Silicon requires that.
Deactivate and reactivate the plugin afterwards:
the copy writes through the bundle's own inode, which is the only swap a running host can see, and a rescan does not reload a plugin that is already loaded.

```sh
# Build the current checkout — branch or main — and load it. One shot.
./update-plugin.sh

# Load a build that already exists, without building anything.
./load-plugin.sh              # menu of every worktree's build
./load-plugin.sh --list       # print the table, load nothing
./load-plugin.sh <branch>     # load that branch's build (substring ok)
```

The two differ in one way:
`update-plugin.sh` builds, `load-plugin.sh` only copies.
That split exists because the bundle slot is shared.
With several branches in flight, each build stays in its own worktree.
You then pick which one goes live, rather than having them overwrite each other.

Both also install `harmonigraph-offline`, which is a second slot and the one that goes quietly out of date.
Video export runs in that binary, and it draws through the same UI crates the editor does —
so a change to any pane changes an mp4 too, even though nothing under `crates/harmonigraph-offline/` was touched.
`update-plugin.sh` rebuilds it every time;
`load-plugin.sh` copies whatever the worktree holds and warns when that predates the branch's HEAD.
A build made with `cargo build --release -p harmonigraph-plugin` alone leaves it behind, and the symptom is an export drawn the old way while the plugin window shows the new one.

## Architecture

Dependencies point strictly downward;
the fun layers never touch plugin plumbing.

```
harmonigraph-core        pure logic, no dependencies at all. PitchClass
                         (integer microcents) and Tuning; lattice coordinates;
                         NoteTracker (what sounds now); NoteHistory/NoteRoll
                         (what was played, by pitch and by time); the FFT
                         spectrum analyzer and its spectrogram history.
                         Unit-tested. One module per concern.
harmonigraph-scene       per-frame view model: derive_scene() turns
                         tracker+tuning into NodeInstances; orbit Camera;
                         envelopes; CPU picking. Split style/view/camera/
                         color/derive; see its crate doc.
harmonigraph-render      wgpu renderer as an egui paint callback: instanced
                         billboard nodes, WGSL in src/shaders/lattice.wgsl.
                         *** Skins/effects/shaders iterate here. ***
harmonigraph-perf        the performance overlay's instrumentation, with no
                         egui in it: what a frame cost, the stage table that
                         says what contains what, the window means and peaks
                         the HUD prints, the process's resident memory, and
                         the build tag its build.rs stamps. Also ShellTimings,
                         which a windowed shell fills in and this averages.
                         The overlay that DRAWS it is harmonigraph-ui.
harmonigraph-ui          egui_dock pane shell: Lattice / Tuning / Display /
                         Console / Spectral / Spiral / Notes / Video tabs
                         under src/panes/, where Display carries the Lattice, Analyzer,
                         Colors, Lighting and System settings as five pages
                         behind a picker row rather than as tabs of their own.
                         SharedState (the lattice's hovered node),
                         ParamBackend trait abstracting "where params live".
harmonigraph-take        the recorded input to a visualization: note events and
                         parameter automation on the audio clock, plus the
                         settings a render is composed from. Linked into the
                         plugin, so no GUI stack: serde+ron and
                         harmonigraph-core. See docs/offline-rendering.md.
harmonigraph-record      writes a take while the transport rolls (lock-free
                         rings, transport-jump detection) and drives
                         harmonigraph-offline as a subprocess when one
                         finishes, following its stderr for the progress bar.
                         No plugin API, so it is testable on its own.
harmonigraph-offline     offline video renderer: replays a take headless at an
                         exact frame rate, any resolution, own pane layout,
                         frames piped to ffmpeg. Also the minimal WAV reader
                         and the audio<->MIDI onset alignment, which read a
                         bounce at render time. No window, no DAW, no realtime.
harmonigraph-standalone  eframe dev harness: mock chord progression OR hardware
                         MIDI in (midir, with MPE bend decoding), plus a mock
                         synth feeding the spectrum analyzer.
harmonigraph-plugin      nice-plug shell: params, MIDI + audio → two rtrb ring
                         buffers, custom wgpu egui editor (editor.rs) with
                         host-native window resizing, CLAP/VST3 exports.
```

Data flow in the plugin:
the audio thread converts host MIDI to `NoteEvent`s and pushes them into a lock-free ring buffer;
the GUI thread drains it into the `NoteTracker`, derives a `Scene`, and paints it.
Parameters flow the other way through `ParamBackend` (a `ParamSetter` in the plugin, plain values in the harness), so every pane runs unmodified in both shells.

## Working on visuals

- `harmonigraph-render/src/shaders/lattice.wgsl` — node look, glow, animation.
- `harmonigraph-scene` — colors, envelopes, layout, camera behavior.
- Run the standalone harness; it uses the identical render path. No DAW needed until you're testing host integration.

## Version coupling

`egui-baseview 0.3` pins egui 0.35 / egui-wgpu 0.35 / wgpu 29 / baseview 0.1;
`eframe` and `egui_dock` must match the egui version.
All of this is centralized in the workspace `Cargo.toml` —
bump the whole cluster together.
`vendor/baseview` and `vendor/egui-baseview` both carry local patches (see PATCHES.md).

## License

Copyright (C) 2026 Yan Han.

Harmonigraph is free software:
you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.
See [`LICENSE`](LICENSE) for the full text.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
See the GNU General Public License for more details.

### Exceptions

[`crates/harmonigraph-core`](crates/harmonigraph-core) is **`MIT OR Apache-2.0`**, not GPL.
It is the one general-purpose library here —
dependency-free just-intonation math, Tonnetz coordinates, and note spelling —
and much of that math descends from the permissively licensed [midi_lattice v1](https://github.com/yan-h/midi_lattice), so it stays permissive too.
`ci.sh` enforces the property that justifies the split:
the crate must remain dependency-free.
See [its README](crates/harmonigraph-core/README.md).

The vendored forks under [`vendor/`](vendor) (`baseview`, `egui-baseview`) are likewise **not** covered by the GPL —
they remain under their upstream `MIT OR Apache-2.0` terms, with their own license files in each directory.
See [`PATCHES.md`](PATCHES.md) for what was changed and why.

VST is a trademark of Steinberg Media Technologies GmbH.
