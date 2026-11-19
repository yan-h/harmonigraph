# harmonigraph-clap

Harmonigraph's CLAP shell:
the plugin framework nice-plug, which harmonigraph used to vendor, together with the two registry crates it built on, now owned here (#1814 step 4).

- **`src/`** is nice-plug 0.1.10 (its CLAP wrapper and state glue) and nice-plug-core 0.1.5 (buffers, the editor context, the debug macros), merged into one crate, with the layers #1814 has since replaced.
  nice-plug-core's modules sit at the crate root, where nice-plug's `pub use nice_plug_core::*` used to put them.
- **`src/params.rs`** is harmonigraph's own (#1814 step 5): one fixed table of host parameters, which replaced nice-plug-core's parameter types and nice-plug-derive's `Params` and `Enum` derives.
- **`tests/`** drive an exported fixture plugin through `clap-sys`: the process callback, flush and editor writes (`clap_boundary.rs`), and the auxiliary bounds (`clap_auxiliary.rs`).
  They run in `ci.sh`'s workspace test gate, which names both fixture features (`assert_process_allocs`, `clap-boundary-tests`).

The library keeps the name `nice_plug`, so harmonigraph-plugin's paths did not change with the move.
The later steps of #1814 collapse these layers into a shell written on `clap-sys`, and rename them as they go.

## Where it came from

- **Upstream base:** crates.io nice-plug 0.1.10, nice-plug-core 0.1.5 and nice-plug-derive 0.1.2, with the ISC `LICENSE` from pinned upstream commit `a80e0e267e52027be4bc07480bef77e798d3342c`, which covers all three.
  The upstream README is at that commit.
- **Pruned to the CLAP wrapper** (#1810):
  the VST3 and standalone wrappers, the macOS, Linux and Windows event loops only they constructed, the Win32 window-API branches, and the `zstd` and `unsafe_flush_denormals` features with the code behind them are gone, with the dependencies only they used.
  The flush-to-zero guard went too: every effect it had sat behind `unsafe_flush_denormals`, so on macOS it was a no-op.
  nice-plug-core's nightly-only `simd` feature and its buffer adapters went when the crate became a workspace member, where `--all-features` would select them.
  The local VST3 fixes (#741's bus arrangements, #1128's note-expression lanes) had already been reverted with the VST3 export (#1185).
- **Not re-vendored.** An upgrade is a port of what a newer nice-plug changed in the CLAP wrapper, the state glue and the tests, taking only what is wanted (Yan, 2026-11-17); nothing the prune deleted comes back with it.
  Upstreaming was declined on 2026-07-19.
  No tuning or sequencing policy belongs in this crate.

## What harmonigraph changed

Each change's reason is a comment at the code it explains; this is the map.

- **The process callback and the plugin trait** (`src/plugin.rs`, `src/events.rs`, `src/wrapper/clap/wrapper.rs`, #1814 step 6).
  nice-plug's `Plugin` and `ClapPlugin`, its contexts, its task queue and background executor, its note events and the opt-in walks over a 2,048-cell input pool gave way to one narrowed `Plugin`.
  Per callback the shell calls `begin` with the `Boundary`, writes the callback's parameter events into the table, then calls `process` with the `Block` and `end`: automation never reaches the table before `begin` has read it.
  Nothing splits a block. The transport is the callback's own; a transport event in the host's list reaches the plugin at its sample, and is logged once from the main thread, since Bitwig sends none.
  - The host's list is validated whole before anything applies (present, in the block, in order, long enough for its type), then read lazily by index as often as the plugin walks it.
    A list that fails loses the callback's input: `process` reads no events, its output refuses everything, and the host gets an error.
    An event the plugin does not read (SysEx, MIDI 2, outside the core space, a per-note parameter event) is skipped.
  - `Output::push(value, time)` calls `try_push` and returns the host's answer; keeping the list sorted is the plugin's contract, checked by a `debug_assert`.
  - A flush applies its values to the table at once, and an editor edit writes the table at once; each row remembers the gesture and value the host has not been told, which the shell reports at offset 0 at the next callback or flush, stopping at the first refusal.
  - The main thread is reached by one deferred host callback request, and serves three flags: the setup's service, a latency change, and the transport line.
  - A prepared `Setup` owns every saved field: it validates before the parameters restore, and commits after; the plugin adopts what it commits at its next callback.
    Its `service` reports what it changed as `Serviced`: a modified project is marked dirty, and changed parameter values or text are rescanned, each only when reported.
  - Main-thread lifecycle calls (`init`, `activate`, `deactivate`, `inactive`, `destroy`) keep registry locking and endpoint reclamation outside audio callbacks.
  - The seams also expose the parameter hash (`wrapper::hash_param_id`), serialize the setup's saved fields beside the parameters, and give tests `Wrapper::test_inspect_plugin`.
    `wrapper::allocation_probe` instruments the debug allocation guard on the calling thread, deallocation included, for the exported-factory fixtures; it measures neither RSS nor other threads.

  [The adaptive tuning design](../../docs/adaptive-tuning.md#the-framework-boundary) says what the plugin asks of this boundary.
- **The parameter table** (`src/params.rs`, #1814 step 5).
  Each row is an integer, an enumeration or a linear float; the host sees it from 0 to its step count, and the conversions repeat nice-plug's `f32` arithmetic step for step, so saves and automation read as before.
  A save stores each row's unmodulated value under its id; a restore sets every row by one rule (missing, wrong-typed or an unknown variant takes the default, out of range is clamped) and logs once the ids no row has.
  The editor sets a row through `GuiContext` by its index.
- **Activation notification ordering** (`Wrapper::activate`, #615): a latency is published only during an activation, and a change while active asks for a restart.
- **Auxiliary descriptor bounds and storage sizing** (`Wrapper::buffers` and `BufferManager::create_buffers`, #638), reached by `tests/clap_auxiliary.rs`.
- **The tail**: none before a processed callback, infinite after (`Wrapper::ext_tail_get`), checked by `tests/clap_boundary.rs`.
- **Track info** (`src/wrapper/clap/track_info_adapter.rs`, #1426).
- **Editor host** (`Plugin::editor_host`, #1607, #1609), and `gui.show`/`gui.hide` refused as upstream does (`Wrapper::ext_gui_show`).
- **`nice-assert-no-alloc` without default features**, from the workspace's single entry.
  Its default `disable_release` would delete the allocation guard from any build without debug assertions;
  while nice-plug-core was a registry crate that declared it with defaults, a `[profile.release.package]` override in the root `Cargo.toml` forced the package's debug assertions on instead.
