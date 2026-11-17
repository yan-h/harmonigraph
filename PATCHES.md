# Carried patches against upstream dependencies

Four dependencies carry local patches, wired in via `[patch.crates-io]` in the workspace `Cargo.toml`:
two vendored here, and the editor window's two pinned from a shared repository.
Keep this file current when bumping them.

nice-plug, baseview and egui-baseview were pruned to what Harmonigraph compiles on macOS, plus a no-window baseview platform that lets a Linux session build and test (#1810);
baseview and egui-baseview have since moved to egui-plugin-window, below.
They are no longer re-vendored from upstream releases (Yan, 2026-11-17):
an upgrade is a port of the code kept, taking from a newer upstream only what is wanted,
rather than a fresh registry package with these patches reapplied.
Upstreaming was declined on 2026-07-19, and both upgrade evaluations found nothing upstream had absorbed.
Their manifests are Cargo's normalized form of the published ones, edited in place;
nice-plug's registry `Cargo.toml.orig` went with the prune.
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

## nice-plug — vendored at `vendor/nice-plug/`

- **Upstream base**: crates.io nice-plug 0.1.10, with its normalized manifest, README, sources and tests, plus the ISC `LICENSE` from pinned upstream commit `a80e0e267e52027be4bc07480bef77e798d3342c`.
- **Pruned to the CLAP wrapper** (#1810): the VST3 and standalone wrappers, the macOS, Linux and Windows event loops only they constructed, the Win32 window-API branches, and the `simd`, `zstd` and `unsafe_flush_denormals` features with the code behind them are gone, with the dependencies only they used.
The flush-to-zero guard went too:
every effect it had sat behind `unsafe_flush_denormals`, so on macOS it was a no-op.
The background-thread executor stays, since the CLAP wrapper spawns it, and so do `assert_process_allocs` and `clap-boundary-tests`, which the fixtures and harmonigraph-plugin's tests enable.
- **Activation notification ordering** (`src/wrapper/clap/wrapper.rs`, `activate`): drop the plugin lock and initialization context before marking the wrapper active.
The context publishes the initial latency while activation is still in progress, as the CLAP latency contract requires, instead of requesting a redundant restart from an already-active wrapper.
This fixes the Bitwig offline-export stall measured by #615;
the exported-factory fixture checks one initial latency notification and no restart for a nonzero delay.
- **Production CLAP ownership** (`src/wrapper/clap/{configuration,configuration_adapter,context,input_adapter,performance,performance_adapter,setup,wrapper}.rs`, `src/wrapper/clap.rs`, `src/wrapper/{state,util}.rs` and `src/wrapper.rs`): the configuration-walk and performance opt-ins share one host-input pool.
The ordinary processing walker, configuration walk and performance owner each keep a progress cursor over it, and an owned cell is reused only once all three have passed.
The configuration walk hands the plugin each callback's boundary (`begin`, where it applies its queued edits and fixes the block's configuration), every owned input in order with its exact sample (`observe`), the end of the walk (`group_end`), the actual sub-blocks (`segment`) and lost input (`fault`).
The plugin owns its configuration and how edits reach it;
the owned-parameter mailbox that once carried the tuning axes as host parameters, with its notification, gesture, modulation and restore layers, is gone (#1756).
The retain-and-acknowledge contract that used to sit on top of that pool is gone;
a plugin that cannot take an input reports a bounded failure instead of holding it.
The output side allocates nothing: `Output` is a borrow of the host's `clap_output_events` whose one `push(value, time)` calls `try_push` and returns the host's answer, which is what an opted-in plugin cannot get from legacy `send_event`.
Keeping the list sorted is the caller's contract rather than the wrapper's:
both halves raise a per-callback high-water mark, the plugin never reads it, and the wrapper puts its own parameter output at it so that traffic follows the plugin's rather than interleaving with it by time.
`PARAMETER_OUTPUT_ATTEMPTS` bounds that half at 512 pushes per callback; the plugin bounds its own.
A prepared setup validates before the parameters and fields restore, and commits after; what it commits the plugin adopts at its next input boundary.
A setup's `service` reports what it changed as `Serviced`: a modified project is marked dirty, and changed parameter values or text are rescanned, each only when reported, so a setting kept outside the parameter list marks the project without a rescan, and a parameter whose label names one (the Lattice Map's label names its map) rescans its text.
Main-thread registration, setup service and joined lifecycle hooks keep registry locking and endpoint reclamation outside audio callbacks.
Joined wrapper destruction runs `clap_main_destroy`, which transfers recording ownership;
the plugin retains original recording routes until joined actual output has a publication disposition.
Together these seams expose the parameter hash, reject legacy sends from a performance owner, serialize the setup's saved fields beside the parameters, and select the measured allocator used by the fixtures.
`allocation_probe.rs` instruments the actual debug allocation guard on the calling thread, including deallocation, for exported-factory ownership fixtures;
it does not measure RSS or other threads.
See [the adaptive tuning design](docs/adaptive-tuning.md#the-framework-boundary) for what the plugin asks of this boundary.
- **Auxiliary descriptor bounds** (`src/wrapper/clap/wrapper.rs`): stop both auxiliary-port loops at the host's declared input/output count, before dereferencing the one-past descriptor (#638).
Missing inputs keep the existing silent-buffer fallback;
missing outputs keep the existing skip-processing behavior and leave undeclared storage untouched.
The existing debug diagnostics remain in place.
`src/wrapper/util/buffer_management.rs` sizes all auxiliary input storage to the current callback before copying or clearing it, so omitted inputs cannot expose the shorter slices left by a preceding callback.
This moves the existing resize within preallocated capacity ahead of the presence check;
it is necessary for #638's silent-input fallback to remain safe across variable callback sizes.
`tests/clap_auxiliary.rs` reaches both bounds through an exported stereo plugin with auxiliary input and output ports, including disappearance, return and changing callback lengths.
- **CLAP tail publication** (`src/wrapper/clap/wrapper.rs`): retain only atomic tail samples after each processed subblock.
The full `ProcessStatus` includes an error string and its `AtomicCell` takes a fallback lock;
only the scalar tail is needed across callbacks.
`tests/clap_boundary.rs` checks exported tail values, intermediate subblock publication, immediate statuses and start/reset/error behavior.
- **CLAP track info** (`src/wrapper/clap/track_info_adapter.rs`, `src/wrapper/clap/{setup,wrapper}.rs`): a plugin with a Setup adapter answers `CLAP_EXT_TRACK_INFO` and its draft-era compat id.
`init` and every host `changed` callback read the track name on the main thread, outside the audio lock, and pass it to `Setup::track_name_changed` (`None` when the host supplies none).
The read stops at the first NUL or the end of the host's fixed buffer.
The name is not saved state;
tuning sources follow it unless the user has set a custom name (#1426).
- **Editor host** (`src/wrapper/clap.rs`, `src/wrapper/clap/wrapper.rs`): `ClapPlugin::clap_editor_host` hands the plugin an `EditorHost` before the editor is made, whose `closed` calls `clap_host_gui::closed(true)` so the editor's own window can ask the host to destroy the editor (#1607, #1609).
`gui.show` and `gui.hide` still refuse, as upstream:
Bitwig calls them on every switch of the frontmost app, so answering them hid the editor's own window whenever another app came forward.
`crates/harmonigraph-plugin/src/configuration/tests.rs` checks that `closed` reaches a host's gui extension asking for destruction.
- **VST3 wrapper**: deleted by the prune, since Harmonigraph ships only as a CLAP (#1185).
The local VST3 fixes (#741's bus arrangements, #1128's note-expression lanes) and their fixture had already been reverted with the export.
- **Upgrade**: a port onto this tree, not a re-vendor.
Read what a newer nice-plug changed in the CLAP wrapper, the state glue and the tests, and port what is wanted, keeping activation notification ordering, scalar tail publication, auxiliary descriptor bounds and storage sizing, production configuration/performance/setup seams, track info, the editor host, root wrapper and state glue, the manifest features, the standalone `[workspace]` table and both boundary fixtures.
Nothing the prune deleted comes back with it.
No tuning or sequencing policy belongs in this framework patch.

## baseview and egui-baseview — pinned from egui-plugin-window

The editor window's two crates are shared with otonal and live in their own public repository, [egui-plugin-window](https://github.com/yan-h/egui-plugin-window), whose `PATCHES.md` now holds the patches they carry and how they are upgraded.
The root `Cargo.toml` pins both by commit in `[patch.crates-io]`.
A change to them lands there first and reaches Harmonigraph as a bump of that `rev`, which moves only the two packages' `source` lines in `Cargo.lock`.

Harmonigraph keeps proving the one build that repository's CI cannot:
the plugin's `frame-log` feature, which reaches into egui-baseview and needs the patched wgpu-hal above (`cargo build -p harmonigraph-plugin --features frame-log`).

To try a windowing change against Harmonigraph before pushing it there, point the patch at a local checkout:

```
cargo build -p harmonigraph-plugin \
  --config 'patch.crates-io.baseview.path="<checkout>/baseview"' \
  --config 'patch.crates-io.egui-baseview.path="<checkout>/egui-baseview"'
```

A patch in configuration takes precedence over the manifest's, so this builds the checkout rather than the pin.
It also rewrites the two packages' `source` lines in `Cargo.lock`;
restore that file with `git checkout Cargo.lock` rather than commit it.

## Historical: nih-plug fork (retired)

Before migrating to nice-plug (which supports host→plugin window resizing natively via `ResizeHint`/`Editor::set_size`), this project carried a nih-plug fork implementing that feature:
`yan-h/nih-plug`, branch `host-window-resize`.
The branch is kept for reference but is no longer a dependency.
