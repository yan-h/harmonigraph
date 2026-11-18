# harmonigraph-clap

Harmonigraph's CLAP shell:
the plugin framework nice-plug, which harmonigraph used to vendor, together with the two registry crates it built on, now owned here (#1814 step 4).

- **`src/`** is nice-plug 0.1.10 (its CLAP wrapper, state glue and background-thread executor) and nice-plug-core 0.1.5 (buffers, contexts, the `Plugin` trait, the debug macros), merged into one crate.
  nice-plug-core's modules sit at the crate root, where nice-plug's `pub use nice_plug_core::*` used to put them.
- **`src/params.rs`** is harmonigraph's own (#1814 step 5): one fixed table of host parameters, which replaced nice-plug-core's parameter types and nice-plug-derive's `Params` and `Enum` derives.
- **`tests/`** are nice-plug's CLAP boundary and auxiliary fixtures, which drive an exported plugin through `clap-sys`.
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

- **Production CLAP ownership** (`src/wrapper/clap/{configuration,configuration_adapter,context,input_adapter,performance,performance_adapter,setup,wrapper}.rs`, `src/wrapper/clap.rs`, `src/wrapper/{state,util}.rs`, `src/wrapper.rs`).
  The configuration-walk and performance opt-ins (`ClapPlugin::CLAP_CONFIGURATION`, `CLAP_PERFORMANCE`) share one host-input pool.
  The ordinary processing walker, the configuration walk and the performance owner each keep a progress cursor over it, and an owned cell is reused only once all three have passed.
  - The configuration walk hands the plugin each callback's boundary (`begin`, where it applies its queued edits and fixes the block's configuration), every owned input in order with its exact sample (`observe`), the end of the walk (`group_end`), the actual sub-blocks (`segment`) and lost input (`fault`).
    The plugin owns its configuration and how edits reach it;
    the owned-parameter mailbox that once carried the tuning axes as host parameters is gone (#1756).
    A plugin that cannot take an input reports a bounded failure instead of holding it.
  - The output side allocates nothing: `Output` borrows the host's `clap_output_events`, and its one `push(value, time)` calls `try_push` and returns the host's answer, which legacy `send_event` cannot give.
    Keeping the list sorted is the caller's contract:
    both halves raise a per-callback high-water mark, and the wrapper puts its own parameter output at it, so that traffic follows the plugin's rather than interleaving with it.
    `PARAMETER_OUTPUT_ATTEMPTS` bounds that half at 512 pushes per callback; the plugin bounds its own.
  - A prepared `Setup` owns every saved field: it validates before the parameters restore, and commits after; the plugin adopts what it commits at its next input boundary.
    Its `service` reports what it changed as `Serviced`: a modified project is marked dirty, and changed parameter values or text are rescanned, each only when reported.
  - Main-thread registration, setup service and the joined lifecycle hooks (`clap_main_*`) keep registry locking and endpoint reclamation outside audio callbacks.
    Joined destruction runs `clap_main_destroy`, which transfers recording ownership.
  - The seams also expose the parameter hash (`wrapper::hash_param_id`), reject legacy sends from a performance owner, serialize the setup's saved fields beside the parameters, and give tests `Wrapper::test_inspect_plugin`.
    `wrapper::allocation_probe` instruments the debug allocation guard on the calling thread, deallocation included, for the exported-factory fixtures; it measures neither RSS nor other threads.

  [The adaptive tuning design](../../docs/adaptive-tuning.md#the-framework-boundary) says what the plugin asks of this boundary.
- **The parameter table** (`src/params.rs`, #1814 step 5).
  Each row is an integer, an enumeration or a linear float; the host sees it from 0 to its step count, and the conversions repeat nice-plug's `f32` arithmetic step for step, so saves and automation read as before.
  A save stores each row's unmodulated value under its id; a restore sets every row by one rule (missing, wrong-typed or an unknown variant takes the default, out of range is clamped) and logs once the ids no row has.
  The editor sets a row through `GuiContext` by its index.
- **Activation notification ordering** (`Wrapper::activate`, #615).
- **Auxiliary descriptor bounds and storage sizing** (the process loop in `src/wrapper/clap/wrapper.rs` and `BufferManager::create_buffers`, #638), reached by `tests/clap_auxiliary.rs`.
- **Scalar tail publication** (`Wrapper::last_tail_samples`), checked by `tests/clap_boundary.rs`.
- **Track info** (`src/wrapper/clap/track_info_adapter.rs`, #1426).
- **Editor host** (`ClapPlugin::clap_editor_host`, #1607, #1609), and `gui.show`/`gui.hide` refused as upstream does (`Wrapper::ext_gui_show`).
- **`nice-assert-no-alloc` without default features**, from the workspace's single entry.
  Its default `disable_release` would delete the allocation guard from any build without debug assertions;
  while nice-plug-core was a registry crate that declared it with defaults, a `[profile.release.package]` override in the root `Cargo.toml` forced the package's debug assertions on instead.
