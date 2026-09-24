# GUI cohort feasibility for #968 (2026-09-19)

Compared product revision `6a4d80a5` with the published cohort below.
This is a feasibility result for [#968](https://github.com/yan-h/harmonigraph/issues/968),
not an implementation launch.

Recommendation: retain the current product stack for now; the published-version trigger is met and should be recorded as investigated, but this is not a low-maintenance dependency bump. The numbered resize and texture-delta patches have credible structural replacements, delayed deadlines also have an upstream equivalent, and the trivial WgpuSetup re-export can be avoided with a direct typed import. Most product-relevant obligations remain, including additional local source obligations absent from PATCHES.md's numbering. No product source, manifests, lockfile or Metal corpus were changed by this investigation.

## Exact published cohort

Official sparse-index entries were freshly fetched and each archive SHA-256 checked against its entry before extraction. Current non-yanked stable releases: egui-baseview 0.7.2 (`f197eade3efd9bfe76b9cbf8357beb6a48e0a2ac262901b0f52c736e2710fdaf`), baseview 0.3.4, egui/egui-wgpu/eframe 0.36.2, egui_dock 0.21.1, wgpu-hal 30.0.1; nice-plug 0.4.2 and nice-plug-egui 0.5.1 were inspected separately. Fresh registry entries and verified archives are retained locally under `/tmp/gui-feasibility-968`;
the primary source links below identify the published inputs.

The actual scratch resolver selected egui-baseview 0.7.2, baseview 0.3.4, egui/egui-wgpu 0.36.2, wgpu/wgpu-core/wgpu-hal/naga 30.0.1. The published egui-baseview requirements remain ^0.36.1, ^30 and ^0.3.4 as #968 recorded. This uses published archives, never a moving repository branch.

Primary sources: [registry entry](https://index.crates.io/eg/ui/egui-baseview),
[verified archive](https://static.crates.io/crates/egui-baseview/egui-baseview-0.7.2.crate),
[published window code](https://docs.rs/crate/egui-baseview/0.7.2/source/src/window.rs),
[renderer](https://docs.rs/crate/egui-baseview/0.7.2/source/src/renderer/wgpu/renderer.rs),
and [baseview macOS view](https://docs.rs/crate/baseview/0.3.4/source/src/platform/macos/view.rs).

## Patch disposition

“Still needed” means the local behavior lacks an upstream equivalent on source inspection, not a fresh native reproduction of every original defect. “Adaptation” means preserve the requirement but rewrite around the new API. “Uncertain” explicitly withholds retirement. Runtime evidence for the two nominated candidates is below.

| Local baseview obligation | Disposition against 0.3.4 | Exact source evidence / remaining work |
|---|---|---|
| 1 Common-mode timer | Still needed | `src/wrappers/appkit/timer.rs:4,23,36` still uses kCFRunLoopDefaultMode. |
| 2 Cursor ownership | Uncertain; partial upstream replacement | New `platform/macos/cursor.rs:9-62` retains desired/current cursor, applies immediately while inside, and `view.rs:633-643` handles cursorUpdate. No resetCursorRects ownership override. Validate first-click, tracking-area/cursor-rect rebuild and host overlap in Bitwig before deciding equivalence; do not blindly reapply the old addCursorRect fix. |
| 3 Occlusion event/immediate frame | Adaptation | No Occluded event or occlusion observer in new event/handler contract; key-change observer at `view.rs:155ff` only. Need a new hook plus recovery in egui-baseview. |
| 4 Configurable/rearmable pacing + display FPS | Adaptation | `view.rs:143` still hardcodes 0.015; new WindowContext API has no set_frame_interval/display_max_fps. |
| 5 Withheld drag-time pointer exit | Still needed | `view.rs:628-631` immediately emits CursorLeft with no pressedMouseButtons gating. |
| 6 Repair lost button release | Still needed | Timer invokes frame directly; no pressedMouseButtons reconciliation, two-tick repair, or duplicate-release suppression. Preserve standalone test workspace wiring when vendoring. |

| Local egui-baseview obligation | Disposition against 0.7.2 | Exact source evidence / remaining work |
|---|---|---|
| 1 Physical/logical Queue resize | Upstream structural equivalent, native candidate | Queue is removed. `window.rs:384,566-569,713-739` derives logical size from physical / total scale; `621-636` sends explicit Size::Logical and multiplies egui zoom once. Do not port the two old conversions mechanically. Native candidate result below; real host resize integration remains separate. |
| 2 Force paint for texture deltas | Uncertain retirement; structural replacement plus wakeup adaptation | `535-558` gates before producing FullOutput; `650-658` always passes produced output to renderer. `renderer/wgpu/renderer.rs:174-188` drains all per-ID delta lists. An idle set_fonts/load_texture by itself does not request repaint (`egui context.rs:2106-2120,2390-2411`); shell must wake it. No evidence of permanent delta loss in this topology. Surface-refusal uploads remain patch 4's problem. |
| 3 Successful-present retry + re-expose | Adaptation | render still returns unit (`renderer.rs:155ff`), pre-UI gate consumes deadline before acquisition (`window.rs:542-552`); no baseview occlusion hook. Need success/failure result, retry scheduling and re-expose wake. |
| 4 Flush pending uploads on refusal | Still needed | Queue writes/update_buffers precede acquire (`174-207`); Occluded/Timeout returns without submit (`212`), reconfigure branch also returns (`224-236`). Source-retention finding, not a repeated RSS failure here. |
| 5 Earliest delayed repaint deadline | Upstream equivalent | Request callback holds earliest deadline (`window.rs:363-375`), gate clears only when due (`542-552`), and skipped ticks do not rebuild UI/rebase deadline. Runtime delayed wake below. |
| 6 Frame timer plumbing | Adaptation | Queue removed; use Frame/WindowContext plus adapted baseview pacing capability. RepaintNotifier only requests a repaint; it is not a frame-rate cap. |
| 7 WgpuSetup re-export | Adaptation with local patch avoidable | Not exported by egui-baseview, but public `egui_wgpu::WgpuSetup` exists (`egui-wgpu src/lib.rs:28`). Current plugin egui-wgpu dep is optional (startup-probe): direct typed import can replace the fork export by adjusting app dependency activation. |
| 8 Tessellation/GPU + extended stage metrics | Adaptation | No equivalent measured API in Frame or renderer; preserve tess/egui_gpu/acquire/tick/render/upload/texture/encode/submit/geometry counters and nonblocking timestamp strategy. |
| 9 update_buffers split + avoid per-frame surface reconfigure | Still needed | Renderer still tests msaa_texture_view.is_none() unconditionally (`196-200`), while filling it only for samples >1 (`128`). Retain corrected guard and separate timing. |
| 10 Layer resize transaction/implicit actions | Adaptation | No layer_present module/hooks/deps. New surface construction is safe platform_handle-based rather than the old raw-window conversion, so hooks must use new surface lifecycle. Validate host geometry transaction and costs. |
| 11 Preserve last pointer position | Still needed | `window.rs:843-845` sets pointer_logical_pos None on CursorLeft while ButtonReleased (`795ff`) still needs Some. |
| 12 Occlusion layer hide/content-coupled unhide | Adaptation | No equivalent; requires baseview occlusion callback plus successful acquire/present signal. Maintain recovery if unocclusion event is lost. |
| 13 Publish managed font texture in CallbackResources | Adaptation | Published renderer only updates textures then buffers (`174-195`); no font texture publication. egui 0.36 changes per-texture delta representation; port at the post-upload/pre-prepare seam with matching wgpu 30 handles. |
| 14 4096 atlas ceiling | Still needed | Renderer max_texture_side is full device limit (`84-90`), placed directly in RawInput (`window.rs:405`); no 4096 cap. |
| 15 SharedGpuContext/reopen | Adaptation | Renderer::new creates a fresh Instance each time (`57-75`), no retained context/loss check. WgpuSetup::Existing can help implement retention but alone is not this contract; instance/surface/adapter/device compatibility and loss eviction must remain correct. |
| 16 Every modifier from the event | Upstream equivalent | `window.rs:507-533` builds all five fields from the set each mouse and key event carries (`771-850`), Meta as `mac_cmd` and `command` on macOS. Nothing to port. |
| Unnumbered: SizeSource | Adaptation, host-path test required | Actual local `window.rs:21-48,847ff` polls outside host size before RawInput. Absent upstream. New baseview has synchronous resize + resized callback/host callbacks; adapt nice-plug set_size/request_resize and prove app- and host-driven geometry adopts before layout. This is separate from point/pixel arithmetic. |
| Unnumbered: surface refusal backoff | Adaptation | Local wants_render/after_render (`633-817`) bounds refused submissions, retries missing events; absent upstream. A pre-UI gate does not bound an animating app requesting repaint every frame while acquire refuses. Preserve bounded submit/retry contract alongside patch 4. |

Actual local source diff versus checksum-published 0.3.0: egui-baseview src has 1,458 insertions / 48 deletions across 5 files; baseview versus 0.1.4 has 612 insertions / 58 deletions across 10 files. Counts are inventory, not estimated port cost. Metrics extensions, SizeSource and refusal backoff show why a stale “all 17 patches” count is not a current decision.

## Other vendored boundaries

| Obligation | Disposition / published evidence |
|---|---|
| wgpu-hal generic Metal library provider | Adaptation to 30.0.1; no upstream provider. `src/metal/device.rs:162ff` still generates MSL then directly calls newLibraryWithSource_options_error around 280; no shader_library module. Keep three-site provider boundary; audit new Naga/MSL options and native compiler options before port. |
| nice-plug activation ordering | Still needed if moving to 0.4.2: wrapper marks is_activated inside activation loop (~1781), then drops activate_context only at 1807. |
| nice-plug production CLAP ownership/configuration/performance/setup | Adaptation if moving; no corresponding custom seams or source modules in 0.4.2. Preserve shared host-input ownership, bounded output, setup/main-thread destruction contracts and exported-factory fixtures. |
| nice-plug auxiliary descriptor bounds/storage sizing | Still needed if moving: clap/wrapper.rs:2065,2089 still use `>` rather than `>=`; util/buffer_management.rs:295 resizes inside input-present path. |
| nice-plug VST3 bus arrangement | Still needed if moving: vst3/wrapper.rs:858,876 main/aux offsets remain reversed; 877ff reads main output unconditionally. |

Sources: [published CLAP wrapper](https://docs.rs/crate/nice-plug/0.4.2/source/src/wrapper/clap/wrapper.rs) and [VST3 wrapper](https://docs.rs/crate/nice-plug/0.4.2/source/src/wrapper/vst3/wrapper.rs). These are source comparisons, not reruns of the local exported-factory fixtures against 0.4.2.

## Type/API/toolchain/corpus consequences

- egui/egui-wgpu/eframe/egui_dock all move together to 0.36.2/0.21.1; Rust >=1.95 is required by their published manifests. The workspace is pinned to 1.92; a scratch 1.95 toolchain was installed under /tmp solely to run the prototype.
- egui-baseview removes closure/Queue/open_parented shape in favor of App/Frame/Window::create and RepaintNotifier. The app UI no longer runs every timer tick. The current product drains state and schedules work inside its update closure: migration must explicitly preserve audio/UI wakeups and idle polls. Retiring forced-render patch 2 does not mean that shell work can remain unchanged.
- baseview 0.3 uses dpi::Size variants and raw-window-handle 0.6 HasWindowHandle; current editor uses 0.5 HasRawWindowHandle adapter. Mixed versions can compile independently but their Rust handle/UI/GPU types are not interchangeable. Rework adapter and resize threading/host contract.
- nice-plug upgrade is not mathematically required by egui-baseview. Current product implements its own Editor and does not depend on nice-plug-egui; current nice-plug baseview dependency is optional behind standalone. Retaining patched nice-plug 0.1.10 while implementing an explicit 0.6 parent-handle bridge is a possible narrower route, not proven by this prototype. Otherwise use current nice-plug 0.4.2/core cohort and rebase its separate audio obligations; do not silently replace the heavily patched audio framework merely to align version numbers.
- wgpu 29 and 30 handles/callbacks cannot be passed across the boundary. Render crate gets wgpu through egui-wgpu; update offline, standalone, callbacks, native layer access and hot-reload naga together. Published textures_delta.set changed to lists per texture ID, so existing uploads/publication code needs adaptation.
- A 29.0.4 patch package does not satisfy ^30: a manifest bump would bypass the vendored wgpu-hal provider unless it is ported. The provider includes package version in BACKEND_VERSION and hashes exact MSL/options; 30.0.1 must regenerate the Metal corpus through production constructors, then strict catalog, offline production paths, real-path goldens, storage bindings, lifecycle/reopen, and performance validation. No corpus was regenerated here. Existing provider cold-start/reopen evidence favors preserving it.

## Runtime probe

Completed scratch build and native execution.
The [reproduction recipe](evidence/gui-cohort-feasibility/README.md) preserves the harness and logging patch;
the [runtime transcript](evidence/gui-cohort-feasibility/probe-final.log) is the definitive measurement.
The exact resolver lock remains local at `/tmp/gui-feasibility-968/probe/Cargo.lock`;
the recipe records its hash and resolved cohort.
The original compile/run command was:

```sh
RUSTUP_HOME=/tmp/gui-feasibility-968/rustup \
CARGO_HOME=/tmp/gui-feasibility-968/cargo RUSTC_WRAPPER='' \
cargo +1.95.0 run -j2 --manifest-path /tmp/gui-feasibility-968/probe/Cargo.toml
```

The published egui-baseview copy was modified ONLY under /tmp to log each real update_texture, surface-acquire outcome and present. No behavioral renderer patch was applied. The GPU_UPLOAD line is just before the actual update_texture call (successful return is evidenced by the later acquire log); it means the real renderer consumed/queued the delta, not that the GPU finished it. Native probe opens its own temporary baseview host with an egui-baseview child; it never touches Bitwig or the shared plugin slot. Explicit child+host close is requested at 12 seconds, with a 15-second process watchdog because this standalone NSApplication run loop did not return after close. This is not a lifecycle/reopen qualification.

Observed outcomes:

1. Native Retina scale was 2.0 and egui zoom was 1.5. Requested initial 300×160 egui points yielded 900×480 physical / 450×240 native logical; total pixels-per-point was 3. App InnerSize(320,180) yielded 960×540 physical and resized callback 320×180 egui logical, with the next UI input 320×180. At t=9s the temporary host called child.resize(PhysicalSize(1080,600)); the callback and next UI input were 360×200 egui logical, native logical was 540×300. Thus the unit conversion works on both app-requested logical resize and externally requested physical child resize without an extra Retina multiplier.
2. After startup stabilized at three UI calls, a worker mutated fonts and allocated a red 16×16 image without request_repaint. Two seconds later the UI count remained exactly three. Explicit request_repaint then ran UI 4 and real renderer update_texture calls for Managed(1) 16×16 plus a full Managed(0) 8192×64 atlas. The pending deltas survived two seconds of idle ticks. A subsequent green image mutation plus request_repaint_after(500ms) ran UI 6 at t=7.561s (roughly the due half-second after the t≈7.06 request), and queued the 16×16 update. This supports the new gating topology and the earliest deadline behavior; it proves the shell needs a wakeup for off-frame mutations.
3. **All eight acquire attempts reported ACQUIRE_OCCLUDED. No PRESENT was logged.** This environment/run did not provide visible presentation; no pixel readback or glyph/image visual correctness is claimed. Texture upload consumption is proven, completed GPU submission/presentation is not. It also exercises the published no-submit early-return path but does not quantify RSS accumulation or a visible ghost. To retire texture patch 2 confidently for the product, repeat the same quiescent font/image test in a visible parented Bitwig-like host and verify presented pixels after explicit wakeup, including surface refusal/recovery.
4. The first UI Context::content_rect diagnostic returned 6666.7×6666.7 before the initial queued zoom/viewport state settled; the next UI input and all later sizes matched. This is a reason not to claim first-frame layout correctness from native window size alone. We did not investigate whether this is a Context query timing effect or an upstream first-pass defect. No product defect is asserted from it.

The local earlier logs (`/tmp/gui-feasibility-968/probe.log`) include compilation and earlier runs; use `probe-final.log` for the final evidence. No failed native test was disguised as a passing migration check. No broad renderer rebase, plugin build, full migration or Metal generation was attempted.
