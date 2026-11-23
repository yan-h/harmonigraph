# harmonigraph-clap

Harmonigraph's CLAP shell:
the plugin framework nice-plug, which harmonigraph used to vendor, together with the two registry crates it built on, now owned here (#1814 step 4).

- **`src/`** is nice-plug 0.1.10 (its CLAP wrapper and state glue) and nice-plug-core 0.1.5 (the debug macros), merged into one crate, with the layers #1814 has since replaced.
  nice-plug-core's modules sit at the crate root, where nice-plug's `pub use nice_plug_core::*` used to put them.
- **`src/params.rs`** is harmonigraph's own (#1814 step 5): one fixed table of host parameters, which replaced nice-plug-core's parameter types and nice-plug-derive's `Params` and `Enum` derives.
- **`tests/`** drive an exported fixture plugin through `clap-sys`: the process callback, flush and editor writes (`clap_boundary.rs`), and the auxiliary bounds (`clap_auxiliary.rs`).
  They run in `ci.sh`'s workspace test gate, which names `clap-boundary-tests`, the fixture feature that turns on `assert_process_allocs` too.
- **`src/testing.rs`**, behind `clap-boundary-tests`, is the one CLAP host those fixtures and harmonigraph-plugin's entry-point suites run on (#1868): it answers only the extensions a suite turns on, and drives a plugin through the C ABI.

The library is `harmonigraph_clap`, the package's own name, and its logging and assertion macros are `shell_*!` (#1814 step 7); step 4 had kept nice-plug's names so the plugin's source moved untouched.
The later steps of #1814 collapsed these layers into a shell written on `clap-sys`.

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
  - The main thread is reached by one deferred host callback request, and serves four flags: the setup's service, a latency change, the transport line, and the line for a restart of the host's steady counter.
  - A `Setup` owns every saved field: it restores them after the parameters, and nothing in them refuses a load; the plugin's audio adopts what it installs at its next callback.
    Its `service` reports what it changed as `Serviced`: a modified project is marked dirty, and changed parameter values or text are rescanned, each only when reported.
  - Main-thread lifecycle calls (`init`, `activate`, `deactivate`, `inactive`, `destroy`) keep registry locking and endpoint reclamation outside audio callbacks.
  - The seams also expose the parameter hash (`wrapper::hash_param_id`), serialize the setup's saved fields beside the parameters, and give tests `Wrapper::test_inspect_plugin`.
    `wrapper::allocation_probe` instruments the debug allocation guard on the calling thread, deallocation included, for the exported-factory fixtures; it measures neither RSS nor other threads.

  [The adaptive tuning design](../../docs/adaptive-tuning.md#the-framework-boundary) says what the plugin asks of this boundary.
- **The parameter table** (`src/params.rs`, #1814 step 5).
  Each row is an integer or a linear float; the host sees it from 0 to its step count, and the conversions repeat nice-plug's `f32` arithmetic step for step, so saves and automation read as before.
  A save stores each row's unmodulated value under its id; a restore sets every row by one rule (missing or wrong-typed takes the default, out of range is clamped) and logs once the ids no row has and once those it set to the default for a wrong-typed value (`Table::defaulted`).
  A saved value that does not decode, such as one under a `ParamValue` variant this build no longer has, is `ParamValue::Unread`, wrong-typed for every row, so it costs that value alone (#1886).
  The editor sets a row through its `EditorHost` by its index.
- **Activation notification ordering** (`Wrapper::activate`, #615): a latency is published only during an activation, and a change while active asks for a restart.
- **The audio** (`src/audio_setup.rs`, #1814 step 6).
  A plugin declares one fixed `Ports`: main input and output channels, and named auxiliary inputs.
  A callback's `Audio` is the main output's channels, holding the main input (copied unless the host processes in place), and each auxiliary input's channels, copied into storage sized at activation, or silence for a port or channel the host left out.
  nice-plug's `Buffer`, `AuxiliaryBuffers`, buffer manager and `AudioIOLayout` went, and auxiliary outputs with them, which no plugin declared.
- **Auxiliary descriptor bounds and storage sizing** (`audio_setup::audio`, #638), reached by `tests/clap_auxiliary.rs`.
- **The tail**: none before a processed callback, infinite after (`Wrapper::ext_tail_get`), checked by `tests/clap_boundary.rs`.
- **Track info** (`src/wrapper/clap/track_info_adapter.rs`, #1426).
- **The editor** (`src/editor.rs`, #1814 step 7).
  A plugin's `Editor` opens under the host's `NSView`, reports its size in logical points, and says whether the host may resize it, freely on both axes; the shell embeds it only in an AppKit view and refuses `gui.set_scale`, since macOS scales.
  The plugin is given its `EditorHost` before its editor (#1607, #1609): the editor's parameter edits by row, its resize requests and its own window's close go through it, and it holds the shell weakly, so a call after the plugin is destroyed does nothing.
  nice-plug-core's `Editor`, `GuiContext`, `ParentWindowHandle`, `PluginApi`, its scale factor, resize hints and host key routing went, with the debug gesture checker; neither plugin used them.
  `gui.show`/`gui.hide` are refused (`Wrapper::ext_gui_show`).
- **`nice-assert-no-alloc` without default features**, from the workspace's single entry.
  Its default `disable_release` would delete the allocation guard from any build without debug assertions;
  while nice-plug-core was a registry crate that declared it with defaults, a `[profile.release.package]` override in the root `Cargo.toml` forced the package's debug assertions on instead.
