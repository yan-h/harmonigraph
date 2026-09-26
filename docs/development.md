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
CI also sets `HARMONIGRAPH_REQUIRE_FFMPEG=1` and installs ffmpeg for the group that runs `harmonigraph-offline`'s tests,
so the real encoder and export-length tests fail rather than skip without `ffmpeg` and `ffprobe`.

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

# Bring a pushed PR (e.g. from a cloud session) into a local worktree and build it.
./build-pr.sh 1079            # rerun after new pushes to fast-forward and rebuild
./build-pr.sh 1079 1081       # several PRs in one go
./build-pr.sh --load 1079     # build and load in one step
```

A cloud session builds on a remote machine and pushes only the branch,
so its PR has no build `load-plugin.sh` can see until `build-pr.sh` makes one locally.
It checks the branch out under `.claude/worktrees/`, where `.claude/reclaim-worktrees.sh` cleans it up like any session's.

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

## Editor layout

The workspace has three independently foldable sections:
Lattice,
Analyzer (or Spiral),
and Settings.
Settings contains Tuning,
Lattice,
Analyzer,
Colors,
Video,
System and Console tabs;
those that do not fit the column move into a trailing overflow menu.
The Analyzer tab's Dock setting places Analyzer to the right of Lattice or below it,
beside Spectrum edge;
each arrangement remembers its own sizes.
Right-clicking Lattice or Analyzer offers a link to its settings tab,
so each setting still has one home.
Dividers resize adjacent visible sections without allowing rearrangement.

Folding requests a smaller outer window while retaining the other picture's dimensions.
In the stacked arrangement this changes height,
and Settings follows the remaining picture's height.
Folding both stacked pictures leaves vertical reopen rails beside Settings.
If the host refuses a resize or the editor reaches its minimum size,
the visible sections fit the available window without overwriting their remembered sizes.

The layout lives in `harmonigraph-ui/src/workspace.rs`,
separate from picture rendering and appearance.
Old saved dock arrangements reset to the fixed default;
camera and appearance settings still load.

## Dependency coupling

`egui-baseview 0.3` pins egui 0.35,
egui-wgpu 0.35,
wgpu 29 and baseview 0.1.
`eframe` and `egui-baseview` must use the same egui version.
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

The [long-term maintainability plan](maintainability-plan.md) records the proposed investigation,
current focus and decisions for work intended to reduce the attention that future maintenance needs from Yan.

## Shared agent skills

Shared personal skills come from your local [agent-config](https://github.com/yan-h/agent-config) checkout,
independently of this repository.
Install `audit-merges` and `review-and-merge` once for Claude and Codex:

```sh
mkdir -p ~/.claude/skills ~/.agents/skills
ln -s ~/projects/agent-config/skills/audit-merges ~/.claude/skills/audit-merges
ln -s ~/projects/agent-config/skills/audit-merges ~/.agents/skills/audit-merges
ln -s ~/projects/agent-config/skills/review-and-merge ~/.claude/skills/review-and-merge
ln -s ~/projects/agent-config/skills/review-and-merge ~/.agents/skills/review-and-merge
```

Use your checkout's actual path,
and preserve any existing skill before replacing it.
For a custom Claude configuration directory,
use its `skills/` directory instead of `~/.claude/skills`.
Update the `agent-config` checkout to update shared skills across projects;
start a new agent session to refresh its skill catalog.
Project-specific skills remain in `.claude/skills`.
The shared skill is optional for development and CI,
but must be installed before Yan invokes a merge audit.

When upgrading an existing checkout that still has the old submodule,
run `git submodule deinit -- .shared-skills` before pulling the removal commit.
Do not force it if Git reports local changes;
preserve those changes first.
This avoids leaving the populated directory behind as untracked files.
Older worktrees can keep their own pinned copy until they are retired;
the Git checkout hook and reclaimer still support them.
