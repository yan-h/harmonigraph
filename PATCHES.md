# Carried patches against upstream dependencies

Three dependencies carry local patches, wired in via `[patch.crates-io]` in the workspace `Cargo.toml`:
wgpu-hal vendored here, and the editor window's two pinned from a shared repository.
Keep this file current when bumping them.

baseview and egui-baseview were pruned to what Harmonigraph compiles on macOS, plus a no-window baseview platform that lets a Linux session build and test (#1810), and have since moved to egui-plugin-window, below.
They are no longer re-vendored from upstream releases (Yan, 2026-11-17):
an upgrade is a port of the code kept, taking from a newer upstream only what is wanted,
rather than a fresh registry package with these patches reapplied.
Upstreaming was declined on 2026-07-19, and both upgrade evaluations found nothing upstream had absorbed.
wgpu-hal is still the unmodified package plus two sites, and is re-vendored as its own section says.

## wgpu-hal — vendored at `vendor/wgpu-hal/`

Based on the unmodified crates.io `wgpu-hal` 29.0.4 package, retaining its licenses and a standalone `[workspace]` table.
It carries two sites, both in `src/metal/mod.rs` and both serving diagnostics:

- `Buffer` gains a public `raw_handle()`, the twin of the one `Texture` already has upstream.
  The GPU memory record (`harmonigraph_render::gpu_memory`), which feeds the perf overlay's GPU-memory row, reads each buffer's liveness and `allocatedSize` through it.
- `set_present_observer` installs a process-wide hook `Queue::present` calls with the present's command buffer and drawable just before the commit (#1551).
  egui-baseview's `frame-log` feature installs it to read `GPUStartTime`/`GPUEndTime` and the drawable's `presentedTime`, which no wgpu API reaches because the drawable is private to the hal.
  With no observer installed it costs one `OnceLock` read per present.

Neither changes generated MSL or a compiler option.
A third site, a provider that served precompiled Metal libraries from a committed corpus, went with that corpus (#1746):
shaders now compile from source through Metal's own cache, and the editor's loading job starts at plugin activation to hide the cold compile.

**Upgrade:** replace the upstream package, retain the workspace exclusion, and reapply only those two sites (dropping `Buffer::raw_handle` if upstream has gained it).
These two diagnostics are now the whole reason the crate stays vendored.

## nice-plug — owned as `crates/harmonigraph-clap/`

nice-plug, nice-plug-core and nice-plug-derive are no longer a patched dependency:
they moved into the workspace as `crates/harmonigraph-clap` (#1814 step 4).
Its [README](crates/harmonigraph-clap/README.md) holds where they came from, what harmonigraph changed in them and how they are upgraded,
and each change's reason is a comment at the code it explains.

## baseview and egui-baseview — pinned from egui-plugin-window

The editor window's two crates are shared with otonal and live in their own public repository, [egui-plugin-window](https://github.com/yan-h/egui-plugin-window), whose `PATCHES.md` now holds the patches they carry and how they are upgraded.
The root `Cargo.toml` pins egui-baseview by commit in `[patch.crates-io]`, and egui-baseview brings baseview by path from the same commit.
Harmonigraph names baseview only as `egui_baseview::baseview`, so it carries no baseview pin of its own that could drift to a second copy.
A change to them lands there first and reaches Harmonigraph as a bump of that `rev`, together with `plugin-own-window`'s below, which moves only the three packages' `source` lines in `Cargo.lock`.

The editor's own window comes from the same repository and commit:
its crate `plugin-own-window`, shared with otonal, which carries no patch since it has no upstream.
Its `rev`, under `[workspace.dependencies]` in the same file, moves with the two above.
What it took from Harmonigraph's former copy, `crates/harmonigraph-plugin/src/editor/pop_out.rs`, is in that repository's `PATCHES.md`;
that file is now the adapter, which gives the window its name, its size limits, the host to tell of its close button, and what a move writes into the editor's saved size.

Harmonigraph keeps proving the one build that repository's CI cannot:
the plugin's `frame-log` feature, which reaches into egui-baseview and needs the patched wgpu-hal above (`cargo build -p harmonigraph-plugin --features frame-log`).

To try a windowing change against Harmonigraph before pushing it there, point the two pinned crates at a local checkout (egui-baseview then brings that checkout's baseview by path):

```
cargo build -p harmonigraph-plugin \
  --config 'patch.crates-io.egui-baseview.path="<checkout>/egui-baseview"' \
  --config 'patch."https://github.com/yan-h/egui-plugin-window".plugin-own-window.path="<checkout>/plugin-own-window"'
```

A patch in configuration takes precedence over the manifest's, so this builds the checkout rather than the pin.
It rewrites the packages' `source` lines in `Cargo.lock`;
restore that file with `git checkout Cargo.lock` afterwards rather than commit it.

## Historical: nih-plug fork (retired)

Before migrating to nice-plug (which supports host→plugin window resizing natively via `ResizeHint`/`Editor::set_size`), this project carried a nih-plug fork implementing that feature:
`yan-h/nih-plug`, branch `host-window-resize`.
The branch is kept for reference but is no longer a dependency.
