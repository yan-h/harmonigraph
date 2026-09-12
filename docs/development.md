# Developing Harmonigraph

This is the maintainer reference for building,
testing and loading Harmonigraph from a source checkout.
For the musical introduction and shortest path to trying it,
start with the [project README](../README.md).

## Prerequisites

The Rust toolchain is pinned by [`rust-toolchain.toml`](../rust-toolchain.toml) (1.92).
With rustup installed,
the first build installs it automatically.

`sccache` must be on `PATH` because [`.cargo/config.toml`](../.cargo/config.toml) sets it as the workspace's `rustc-wrapper`.
Without it,
Cargo exits with `could not execute process sccache`.

```sh
brew install sccache
```

Every worktree keeps its own `target/`,
so parallel branches do not wait on a shared build lock.
They share compiled dependencies through sccache instead.
To rule out sccache while diagnosing a build failure,
run a Cargo command with `RUSTC_WRAPPER=""`.

`ffmpeg` is required only to encode a playable video,
not to produce a frame sequence:

```sh
brew install ffmpeg
```

## Everyday commands

```sh
# Full UI and renderer in a plain window, using a mock chord progression
# or any connected MIDI port. No DAW is required.
cargo run -p harmonigraph-standalone

# Workspace tests.
cargo test

# Canonical full gate used by GitHub Actions.
./ci.sh

# Read the plugin's live settings from a saved Bitwig project.
# Close the plugin window and save the project first.
./read-plugin-state.py
```

The standalone harness uses the same UI and render paths as the plugin.
It can accept hardware or virtual MIDI,
and feeds its analyzer with a mock synth rather than external audio.

Enable the tracked pre-push formatting check once per clone:

```sh
git config core.hooksPath .githooks
```

Claude's checked-in `SessionStart` does this automatically.
The same hook path also initialises the pinned shared-skills submodule when Git creates a worktree;
other clients should run the command once after cloning.
The pre-push hook checks formatting only.
GitHub Actions runs the full `./ci.sh` gate for pull requests and pushes to `main`,
split across parallel groups and reported as one `Full CI` check.
A bare local `./ci.sh` runs every group.

CI sets `HARMONIGRAPH_REQUIRE_GPU=1`,
so an unavailable GPU adapter fails renderer,
UI and offline pixel tests instead of skipping their assertions.
Local GPU tests may skip without an adapter;
set the same variable to require one locally.
GPU setup honors `WGPU_BACKEND`,
and an empty value disables all backends for testing the unavailable-adapter path.

## Loading a worktree build in Bitwig

This repository's Bitwig workflow uses one bundle location:
the main checkout's `target/bundled/`.
In a fresh Bitwig setup,
add that directory under **Settings → Locations → Plug-in Locations**.
Create the bundle structure once from the main checkout before using either loader script:

```sh
cargo xtask bundle harmonigraph-plugin --release
```

A branch build is otherwise hidden from Bitwig for two reasons:

- Every worktree has its own `target/`.
- `cargo xtask bundle` chooses the topmost ancestor containing `Cargo.toml`.
  From a nested worktree it can silently bundle the main checkout's source rather than the branch.

After that one-time bootstrap,
use the repository scripts for branch builds.
They copy the executable into the shared bundle slot and re-sign it ad hoc for Apple Silicon.

```sh
# Build this checkout and load it in one step.
./update-plugin.sh

# Load an existing worktree build without building.
./load-plugin.sh              # interactive worktree menu
./load-plugin.sh --list       # show builds without loading one
./load-plugin.sh <branch>     # a unique branch substring is enough
```

`update-plugin.sh` builds;
`load-plugin.sh` only copies.
The split lets every parallel branch retain its own build while the person at the DAW decides which one occupies the shared slot.

`update-plugin.sh` builds and installs a matching `harmonigraph-offline` renderer.
`load-plugin.sh` installs the worktree's renderer when it exists;
otherwise it leaves the installed renderer alone.
The offline renderer draws through the same UI and render crates,
so a change to a pane can change an exported video even when no file under `crates/harmonigraph-offline/` moved.
Building only `harmonigraph-plugin` can therefore leave export on an old picture.
`load-plugin.sh` warns when its renderer predates the branch's `HEAD`.

After installing,
deactivate and reactivate Bitwig's audio engine.
An individual plugin-device toggle,
editor reopen or rescan may leave a grouped plugin-host process and its old mapped code alive.
The loader reports the executable identities it can observe in Bitwig's processes;
the performance overlay's build tag is the final check of what the DAW loaded.

## Architecture

Dependencies point downward;
the rendering and music layers do not depend on plugin plumbing.

```text
harmonigraph-core        Dependency-free pitch and tuning logic; lattice
                         coordinates; live and historical note state; shared
                         pitch axis and spectrogram history.

harmonigraph-analysis    Rolling audio analysis shared by live and offline UI.

harmonigraph-scene       Per-frame scene derivation, camera, styles, envelopes
                         and CPU picking.

harmonigraph-render      wgpu lattice renderer used as an egui paint callback.

harmonigraph-perf        Performance instrumentation and build-tag stamping;
                         the UI crate draws its overlay.

harmonigraph-ui          Shared pane shell and controls for Lattice, Tuning,
                         Display, Console, Spectral, Spiral, Notes and Video.

harmonigraph-take        Recorded note events, parameter automation and the
                         settings needed to reproduce a visualization.

harmonigraph-record      Realtime-safe take writing and offline-renderer launch.

harmonigraph-offline     Headless take replay, WAV input, audio/MIDI alignment
                         and frames piped to ffmpeg.

harmonigraph-standalone  eframe development harness with MIDI input and a mock
                         progression and synth.

harmonigraph-plugin      nice-plug shell, lock-free audio-to-UI ingress,
                         custom editor and CLAP/VST3 exports.
```

In the plugin,
the audio thread converts host MIDI to note events and pushes them through a lock-free ring buffer.
The GUI thread drains the events into the note tracker,
derives a scene and paints it.
Parameters travel through the shared UI's `ParamBackend`,
implemented by the plugin host in the plugin and by plain values in the standalone harness.

See [offline rendering](offline-rendering.md) for the recorded-data path and [adaptive tuning](adaptive-tuning.md) for the Tune/Hub protocol.

## Working on visuals

- `crates/harmonigraph-render/src/shaders/lattice.wgsl` controls the node surface,
  glow and animation.
- `crates/harmonigraph-scene` owns colors,
  envelopes,
  layout and camera behavior.
- `cargo run -p harmonigraph-standalone` exercises the production render path without a DAW.

## Dependency coupling

`egui-baseview 0.3` pins egui 0.35,
egui-wgpu 0.35,
wgpu 29 and baseview 0.1.
`eframe` and `egui_dock` must use the same egui version.
The workspace [`Cargo.toml`](../Cargo.toml) centralizes these versions;
bump the cluster together.

The workspace carries local dependency patches under [`vendor/`](../vendor).
Their provenance,
platform scope and upgrade notes live in [`PATCHES.md`](../PATCHES.md).

## Repository workflow

[`CLAUDE.md`](../CLAUDE.md) is the project contract for agent-managed worktrees,
formatting,
build handoff and draft pull requests.
Almost all implementation is written through LLM coding sessions under Yan's direction and review.
