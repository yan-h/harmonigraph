# Carried patches against upstream dependencies

Four dependencies carry local patches, wired in via `[patch.crates-io]` in the workspace `Cargo.toml`.
Keep this file current when bumping them.

## wgpu-hal — vendored at `vendor/wgpu-hal/`

Based on the unmodified crates.io `wgpu-hal` 29.0.4 package, retaining its licenses and a standalone `[workspace]` table.
The shader-library patch has three sites: `src/metal/mod.rs` exports `shader_library`, `src/metal/shader_library.rs` defines a generic immutable provider, and the normal Naga branch of `Device::load_shader` delegates only source-to-library creation to it.
Normal specialization, MSL generation, entry-point lookup, bounds/reflection metadata and per-device resource ownership are unchanged.
No filesystem, artifact manifest, application dependency or GPU-object cache belongs to the patch.

A fourth, separate site: `src/metal/mod.rs` gives `Buffer` a public `raw_handle()`, the twin of the one `Texture` already has upstream.
The GPU memory record (`harmonigraph_render::gpu_memory`) reads each buffer's liveness and `allocatedSize` through it.
It changes no generated MSL or compiler option, so the shader corpus does not depend on it.

A fifth site, also separate: `src/metal/mod.rs` gains `set_present_observer`, a process-wide hook `Queue::present` calls with the present's command buffer and drawable just before the commit (#1551).
egui-baseview's `frame-log` feature installs it to read `GPUStartTime`/`GPUEndTime` and the drawable's `presentedTime`, which no wgpu API reaches because the drawable is private to the hal.
With no observer installed it costs one `OnceLock` read per present, and it changes no generated MSL or compiler option.

The provider is installed before device creation, so backend-internal shaders use the same path.
Installation is unsafe because wgpu cannot validate arbitrary Metal bytecode; the application must establish exact source/options equivalence and artifact integrity.
Harmonigraph does that with build-time hashes and exact immutable-input lookup.
Unavailable native compiler-option APIs are guarded before access, and unsupported profiles retain source fallback.

**Upgrade:** replace the upstream package, retain the workspace exclusion, and reapply only those five sites (dropping `Buffer::raw_handle` if upstream has gained it).
Inspect all compiler options and changes around the normal reflection boundary, then regenerate the corpus through the real production constructors.
Require strict catalog/real-path golden coverage, storage-binding behavior, native lifecycle checks and rendering-performance validation.
The optional provider is a candidate upstream boundary, not a reason to accumulate renderer-specific logic in the backend.
See [production assets](docs/metal-shader-assets.md) for generation, validation and fallback policy.

## nice-plug — vendored at `vendor/nice-plug/`

- **Upstream base**: crates.io nice-plug 0.1.10, with its normalized standalone manifest, original manifest, README, sources and tests, plus the ISC `LICENSE` from pinned upstream commit `a80e0e267e52027be4bc07480bef77e798d3342c`.
- **Activation notification ordering** (`src/wrapper/clap/wrapper.rs`, `activate`): drop the plugin lock and initialization context before marking the wrapper active.
The context publishes the initial latency while activation is still in progress, as the CLAP latency contract requires, instead of requesting a redundant restart from an already-active wrapper.
This fixes the Bitwig offline-export stall measured by #615;
the exported-factory fixture checks one initial latency notification and no restart for a nonzero delay.
- **Production CLAP ownership** (`src/wrapper/clap/{configuration,configuration_adapter,context,input_adapter,performance,performance_adapter,setup,wrapper}.rs`, `src/wrapper/clap.rs`, `src/wrapper/{state,util}.rs` and `src/wrapper.rs`): the effective-configuration and performance opt-ins share one host-input pool.
The ordinary processing walker, configuration owner and performance owner each keep a progress cursor over it, and an owned cell is reused only once all three have passed.
The retain-and-acknowledge contract that used to sit on top of that pool is gone;
a plugin that cannot take an input reports a bounded failure instead of holding it.
The output side allocates nothing: `Output` is a borrow of the host's `clap_output_events` whose one `push(value, time)` calls `try_push` and returns the host's answer, which is what an opted-in plugin cannot get from legacy `send_event`.
Keeping the list sorted is the caller's contract rather than the wrapper's:
both halves raise a per-callback high-water mark, the plugin never reads it, and the wrapper floors its own parameter and configuration output at it so that traffic follows the plugin's rather than interleaving with it by time.
`PARAMETER_OUTPUT_ATTEMPTS` bounds that half at 512 pushes per callback; the plugin bounds its own.
Prepared nonautomatable setup validates and reserves capacity before parameter/state mutation, then adopts at the enclosing input boundary.
Main-thread registration, setup service and joined lifecycle hooks keep registry locking and endpoint reclamation outside audio callbacks.
Joined wrapper destruction takes the configuration runtime and calls `clap_configuration_retire(unfinished)` before `clap_main_destroy` transfers recording ownership.
The hook disposes unfinished input, command and learning ownership without claiming successful application or advancing a sample prefix;
the plugin retains original recording routes until joined actual output has a publication disposition.
Tune's performance-only opt-in creates no configuration mailbox.
Together these seams expose the parameter hash, reject legacy sends from a performance owner, serialize the overlaid configuration state, and select the measured allocator used by the fixtures.
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
- **VST3 wrapper**: unpatched upstream, and not compiled into Harmonigraph, which ships only as a CLAP and depends on nice-plug without its default `vst3` feature (#1185).
The local VST3 fixes (#741's bus arrangements, #1128's note-expression lanes) and their fixture were reverted with the export, so an upgrade has nothing VST3 to reapply.
- **Upgrade**: replace the vendored upstream files including the license, retain the standalone `[workspace]` table, and reapply activation notification ordering, scalar tail publication, auxiliary descriptor bounds and storage sizing, production configuration/performance/setup seams, track info, the editor host, root wrapper and state glue, manifest features, and both boundary fixtures.
No tuning or sequencing policy belongs in this framework patch.

## baseview — vendored at `vendor/baseview/`

- **Upstream base**: `baseview 0.1.4` from crates.io (the version
egui-baseview 0.3 pins;
wired in via `[patch.crates-io]` in the workspace `Cargo.toml`, which covers both our direct dependency and egui-baseview's).
- **Patch** (3 lines, `src/wrappers/appkit/timer.rs`): register the macOS
run-loop timer in `kCFRunLoopCommonModes` instead of `kCFRunLoopDefaultMode`.
Default-mode timers stop firing while the run loop is in an event-tracking mode (native drags/resizes, menu tracking), freezing plugin GUIs until mouse release.
The timer was upstream's frame timer; since patch 4 it is only the display link's watchdog, and the link itself is added in common modes too.
- **Patch 2** (4 sites: `src/platform/macos/{window,view}.rs`,
`src/wrappers/appkit/view.rs` + `view/implementation.rs`):
own the mouse cursor properly.
`set_mouse_cursor` used a one-shot `addCursorRect`, which AppKit discards on the next cursor-rect rebuild —
after which the HOST window's rects win, so the plugin's cursor appeared to depend on whatever Bitwig had behind the window.
Now the desired cursor is stored and re-asserted from a `resetCursorRects` override, with `invalidateCursorRectsForView` forcing a rebuild on change.
Because cursor rects only activate once the window becomes KEY (first click), a `cursorUpdate:` override (delivered via the tracking area's CursorUpdate | ActiveInActiveApp options) plus direct application on change-while-hovered were added for the freshly opened window as well.
KNOWN LIMITATION:
in Bitwig the cursor still doesn't take until the plugin window is first clicked (the host's not-yet-key window appears not to deliver tracking/cursorUpdate events);
accepted as cosmetic —
everything is correct from the first click on.
- **Patch 3** (5 files: `src/event.rs`,
`src/wrappers/appkit/notification_center.rs`, `src/platform/macos/view.rs`,
`src/wrappers/appkit/view.rs` + `view/implementation.rs`):
new `WindowEvent::Occluded(bool)` (mirroring winit's event), emitted from an `NSWindowDidChangeOcclusionStateNotification` observer filtered to the view's own window.
On re-expose the view is marked `setNeedsDisplay` and, more to the point, a frame is run right there in the notification rather than left to the next display-link callback (patch 4) —
nothing can present while the window is occluded, so what is on screen is the drawable from before it was hidden, and the ghost lasts exactly until one frame gets out.
Waiting for the callback made that up to a frame interval:
measured at 11-14 ms under the ~15 ms frame timer the link replaced, and it scales with the frame divisor, so a frame-rate cap makes it worse;
painting from the notification measures under 1 ms. `trigger_frame` takes the handler borrow fallibly to stay safe against a notification arriving mid-frame (that direction only —
`trigger_event` still borrows infallibly and documents its panic), and the handler drops a notification repeating the state the view is already in, which would otherwise buy an off-cadence frame for nothing.
That state is kept per window attachment (#1070):
`window_visible` is an `Option<bool>`,
and a `viewDidMoveToWindow` override (a new `ViewImpl` method registered in the two appkit wrapper files) clears it to `None` and samples the new window,
so a reparented view reports its new window even when both windows read the same.
The view also samples once at open, right after the handler's initial resize event;
`occlusion_ready` holds the override off until then, because the first `viewDidMoveToWindow` runs during parenting before the handler exists.
That is what lets a window that opens occluded deliver its reveal.
The open-time sample reports the state without the re-expose frame.
Together with the egui-baseview patch below, this fixes the outdated ghost image that stayed on screen after tabbing away from the host and back, until the window was clicked.
macOS only;
other platforms never emit the event.
- **Patch 4** (`src/lib.rs`, `src/window_open_options.rs`, `src/window.rs`,
`src/platform/macos/{view,window}.rs`, `src/wrappers/appkit/view.rs` + `view/implementation.rs`, `Cargo.toml`):
drive frames from the view's display link instead of a fixed timer (#1551).
Upstream's timer fired every 15 ms with no relation to vsync, so a frame was ready whenever it was ready and the display asked when it asked;
at 144 Hz in Bitwig that missed about 4% of refreshes, and a timer at twice the refresh rate did not help.
Now `NSView.displayLinkWithTarget:selector:` (macOS 14) calls a new `displayLinkFired:` method once per refresh of whichever screen the view is on,
from the main run loop in common modes, created once the view is parented and invalidated in `close`.
The link retains the view, so a missed `invalidate` leaks it (the plugin's `editor-startup` probe catches that),
and the callback holds its own reference to the view because a frame can close the window and drop the link's.
`Window::set_frame_divisor` draws on every Nth refresh, paced on the link's `targetTimestamp` rather than by counting callbacks, with the next frame due half a refresh short of N ahead so jitter cannot slip it.
A divisor rather than a rate because the link's `preferredFrameRateRange` rounds to divisors of the panel's rate: measured on a 144 Hz panel, 120 ran at 144 and 90 and 60 at 72.
A link stops while its view is hidden or its window minimized or ordered out (measured; it keeps firing while the window is merely covered),
so the common-mode timer from patch 1 survives as a 10 Hz watchdog that runs a frame whenever the link has been silent for 100 ms:
the frame is also what ages the plugin's notes and lets an armed take end itself while the editor is open but out of sight (its rings are drained by the plugin's input worker, #1566),
and what repairs a stuck button or a withheld exit.
`Window::frame_timing` hands the frame callback the link's timestamps and the divisor it paced the frame at, for egui-baseview's frame log,
and the fire on `Instant`'s clock beside the media clock's (`fired_at`);
`FrameTiming::target_at` puts the target on `Instant`'s clock, refusing one more than 0.1 s from the fire, for the plugin's picture and egui's clock to time the frame by.
The earlier version of this patch made the timer's interval configurable and reported `NSScreen::maximumFramesPerSecond`;
`WindowOpenOptions::frame_interval`, `set_frame_interval`, `display_max_fps` and the interval constants are gone with the timer.
macOS only;
`set_frame_divisor` is a no-op elsewhere, where frames still come from a fixed 15 ms timer, and `frame_timing` returns `None`.
- **Patch 5** (`src/platform/macos/view.rs`): hold the pointer's exit back
while a mouse button is down.
The tracking area carries `EnabledDuringMouseDrag`, so `mouseExited:` arrives mid-drag —
while AppKit goes on sending every `mouseDragged:` and the closing `mouseUp:` to the view the press landed in, wherever the pointer has got to.
Reporting the exit there is what breaks the drag:
a consumer told the pointer is gone stops following it (egui turns `CursorLeft` into `PointerGone`), so a slider dragged a pixel past the window edge let go under the hand still holding it.
The `CursorLeft` is now withheld rather than dropped —
paid from the matching mouse-up, and from the frame tick for the releases that never arrive at all (a press that was never ours, a host that took the release), both gated on `NSEvent::pressedMouseButtons` rather than on a count of the presses this view has seen, which a release delivered elsewhere would leave wrong forever.
`mouseEntered:` clears it and stays silent, so enter and exit remain paired.
macOS only;
the other backends have no drag-time exit.
- **Patch 6** (`src/platform/macos/view.rs`, `Cargo.toml`): report the release
of a button the OS says is no longer held.
A press this view reports is owed a release, and AppKit does not always pay —
a `mouseUp:` handed to a host popup, a modal opening under the finger, a gesture the host takes over —
so the consumer is left believing the button is down forever.
That costs far more than it sounds like, because drag state is global:
egui gates EVERY `ScrollArea` on `dragged_id().is_none()`, so one unreleased press stops the wheel in every scrollable pane at once, with the window focused and the pointer over the pane that will not move.
Patch 5 does not reach it (it waits on an exit that never comes if the pointer never leaves) and neither does a consumer-side guard on lost focus;
both watch the POINTER, and this is the BUTTON.
So the frame tick now compares the buttons the view has reported pressed against `NSEvent::pressedMouseButtons` —
the same authority Patch 5 already trusts —
and synthesises the missing `ButtonReleased`.
A stuck button must read stuck across two ticks (the hardware goes up before a queued `mouseUp:` is dispatched, so one tick cannot tell a fast click from a lost release), a late real `mouseUp:` is then swallowed so a bit yields one release however it is paid, and every button above the first two shares a bit because `otherMouseDown:` reports them all as `Middle` —
so two of THOSE held at once are one bit between them, the second release is dropped as a duplicate, and a stuck one behind an already-released sibling is not repaired.
Neither reaches the consumer, whose button is a bool that the first release has already put up.
Real drags are untouched:
through one the button reads down.
`Cargo.toml` gains an empty `[workspace]` table so `cargo test --manifest-path` reaches the unit tests over the mask arithmetic;
ci.sh runs them.
macOS only.
- **Upgrade**: read the [published-cohort feasibility map](docs/gui-cohort-feasibility.md) first;
baseview 0.4 drives frames from a display link too, but pauses it while idle and has no divisor, watchdog or frame timing.
Download the new crates.io tarball into `vendor/baseview`,
re-apply the `kCFRunLoop*` lines, the cursor-rect ownership patch, the occlusion-event patch with its per-attachment sampling, the display link with its divisor, watchdog and frame timing, the withheld pointer exit, the synthesized release for a stuck button, and the `[workspace]` table that lets the tests run.
- **Upstreaming**: declined (2026-07-19).
Kept as a local patch rather than proposed upstream.

## egui-baseview — vendored at `vendor/egui-baseview/`

- **Upstream base**: `egui-baseview 0.3.0` from crates.io (the RustAudio
fork used by nice-plug).
- **Patch 1** (2 call sites, `src/window.rs`): after a `Queue::resize()`,
the window resize triggered by the physical-size change passed physical pixels to `Window::resize()`, which takes logical points —
on scaled displays the window/view ended up `pixels_per_point` times too large (2x-zoomed, bottom-left-anchored content on Retina).
Convert with `points_per_pixel` at both call sites (build path and `on_frame`).
- **Patch 2** (1 site, `src/window.rs` `on_frame`): texture deltas (font
atlas rebuilds after `set_fonts`, new images) are only uploaded inside `render()`, but frames with no repaint due skip `render()` and drop the deltas permanently —
glyph coordinates then point into a stale atlas (scrambled text).
Force a render whenever `textures_delta` is non-empty.
Only visible for apps that throttle repaints;
always-repaint apps mask it.
- **Patch 3** (`src/window.rs` + both renderers): stale-frame ("ghost")
recovery after window occlusion.
`Renderer::render` now returns whether a frame was actually presented;
a skipped present (occluded window, outdated/lost surface —
the wgpu renderer's early-return paths) no longer consumes the repaint request, so rendering retries instead of freezing on the last presented frame (Patch 17 bounds how often once the refusal is not transient).
The new `baseview::WindowEvent::Occluded(false)` (see the baseview patch above) schedules an immediate repaint so the first frame after re-expose is fresh.
- **Patch 4** (2 sites, `src/renderer/wgpu/renderer.rs` `render`): flush
staged uploads on the surface-not-available early returns.
`render` uploads egui's per-frame vertex/index/texture data (via `update_buffers` /`update_texture`, i.e. `queue.write_*`) BEFORE it acquires the surface texture;
those staging buffers live in wgpu's pending writes and are only reclaimed by a `submit()` (through wgpu-core `pre_submit`).
The Occluded/Timeout and Suboptimal/Outdated/Lost arms returned WITHOUT submitting, stranding that frame's staging buffers.
Combined with Patch 3 (a skipped present retried every timer tick, before Patch 17's backoff), a backgrounded plugin window re-ran `render` ~66×/s and accumulated staging buffers into the *gigabytes* within minutes —
the memory dropped instantly on refocus, when the next presented frame's `submit` finally drained the backlog.
Fix:
each early return now submits `user_cmd_bufs` + the upload encoder (no drawable acquired, so nothing presents —
the window stays frozen while hidden, which is expected —
but pending writes drain and `maintain` runs every tick, so memory stays flat).
Root-caused from a 26 GB balloon while tabbed away from Bitwig.
- **Patch 5** (1 site, `src/window.rs` `on_frame`): make delayed repaints
actually come due.
The repaint deadline was recomputed as `now + repaint_delay` on *every* tick that painted nothing, and egui rebuilds `repaint_delay` from scratch each pass (reset to `MAX` in `begin_pass_repaint_logic`, then the min of that pass's requests) while the UI closure runs on every tick, painting or not.
So a steady `request_repaint_after(N)` re-based the deadline on each tick, and for any N longer than the tick interval (then ~15 ms, the macOS frame timer; now one display refresh) `now` never caught up:
the deadline receded forever and the window painted nothing until an input event or a texture upload forced it.
Every delayed repaint was silently dead —
the idle poll included, which went unnoticed because the plugin shell requests a repaint whenever it drains MIDI, and an idle window has nothing to show anyway.
Fix:
keep the EARLIEST pending deadline rather than overwriting it, and schedule the next one from the instant a frame actually painted instead of leaving it unset (clearing it to `None` cost a whole tick, so every capped interval ran one tick long).
Found while adding the System pane's frame-rate cap, which was first built on exactly this mechanism and did nothing at all without the fix;
the cap is now baseview's frame divisor (Patch 6), and this fix still serves every delayed repaint, the idle poll among them.
- **Patch 6** (`src/window.rs`): pass baseview's frame divisor through, so the
app can pace its own window.
`Queue::set_frame_divisor` sets it from inside a frame (applied after the frame returns, where the `Window` is reachable —
the same deferred shape `resize` already uses).
This is what makes a frame rate setting enforceable:
egui's `request_repaint_after` cannot do it, since egui keeps the SMALLEST delay requested in a pass and any zero-delay `request_repaint` (input event, hover animation, a host draining MIDI) also forces the next pass to zero —
so a cap built on it evaporates exactly when the UI is busy.
Nor can the two be combined: a repaint delay of one frame skips every frame that arrives a hair early, halving the rate (#1551).
`Queue::frame_timing` hands the frame its link timing from baseview patch 4, so the app can time its content by the refresh rather than by when the callback ran,
and egui's own `input.time` is the link's target too (never going back past the last one), so egui's animations step with the picture (#1562).
Before #1551 this plumbed a configurable frame timer interval and the display's maximum rate instead.
- **Patch 7** (2 lines, `src/renderer.rs` + `src/lib.rs`): re-export
`WgpuSetup` alongside `GraphicsConfig`.
`WgpuConfiguration` was already public, but its `wgpu_setup` field cannot be matched without the enum, so there was no way to reach the `device_descriptor` hook and request an extra device feature (we ask for timestamp queries, for the overlay's GPU-time row).
Pure re-export;
no behavior change.
- **Patch 8** (`src/renderer/wgpu/renderer.rs`, `src/window.rs`): measure the
frame's other two halves and hand them back through `Queue` —
`tess_ms` (time in `egui::Context::tessellate`) and `draw_gpu_ms` (GPU time from paint-callback preparation through egui's composite, lattice 3D included, via a timestamp query pair;
named `egui_gpu_ms` and bracketing only egui's own pass before #1074).
Both sat in blind spots:
tessellation runs after the update closure returns, so it is neither the app's own frame time nor GPU time, and a wgpu paint callback's timer brackets only ITS passes, never the 2D UI around them.
A frame cost could live entirely in either and read as zero everywhere.
Since #1074 the opening sample is the beginning of a 1x1 pass placed before `update_buffers`, so callback preparation is inside the bracket,
and the closing sample is the END of egui's render pass, so the bracket cannot close before the composite finishes.
The former tail pass, whose beginning closed the bracket, could read reversed against egui's own beginning;
the new pair measured 0 of 60 reversed on Metal.
Queue-staged uploads and callback-owned command buffers stay outside it.
Readback is a four-step cycle polled with `PollType::Poll`, never `Wait` —
blocking for the number would stall the pipeline being measured.
The resolve waits for `on_submitted_work_done` and goes in its own submission:
resolved with the frame, an M5 Pro under macOS 27 read the closing slot before its stamp landed on 98 of 100 frames,
and the slot kept the last frame's value, so nearly every pair came out reversed and was dropped.
A repeated opening stamp is refused for the same reason.
The cycle duplicates `harmonigraph-render`'s lattice `GpuTimer` (`gpu_timer.rs`), which differs only in where its closing stamp goes;
a fix to either belongs in both.
- **Patch 9** (`src/renderer/wgpu/renderer.rs`, `src/window.rs`): split the
upload reading, then fix what the split exposed.
`last_ubuf_ms` times `update_buffers` on its own, because the surrounding `last_upload_ms` also spans creating the command encoder, taking the renderer's write lock, and the MSAA resize —
none of which are uploads, and two of which are places a frame blocks rather than works.
With those apart, the resize guard turned out to fire every frame.
It tested `msaa_texture_view.is_none()`, but `RendererOptions::default()` sets `msaa_samples` to 0 and `resize_and_generate_msaa_view` only fills that view when it is above 1 —
so the view was never `Some`, and every frame reconfigured the surface to the size it already had.
Rebuilding a swapchain per frame cost 0.5-3 ms, wandered on a ~1 s period as GPU load moved, and was unattributable:
it read as "buffer uploads" on the perf overlay while `update_buffers` itself measured 0.3 ms. The term is now gated on `msaa_samples > 1`;
`width`/`height` start at 0, so the first frame still configures.
- **Patch 10** (`src/renderer/wgpu/renderer.rs` `layer_present` module +
hooks, `Cargo.toml` objc2 deps):
present a resize without the flash (issue #121's fold flicker).
Two artifacts, one layer:
wgpu presents into a `CAMetalLayer` that raw-window-metal adds as a plain (not view-backed) sublayer, synced to the view's root layer from a KVO observer —
so every geometry change it takes carries CA's default quarter-second implicit ease (of the presentation layer only;
the model reports final values, which is why #120's probes saw "nothing animating"), and it takes those changes inside the HOST's transaction, out of reach of any `CATransaction` wrapped around our own resize calls.
A null `actions` dictionary on the layer kills the lookup for every caller.
Second:
a present is decoupled from the transaction carrying the new bounds, so the resize commit shows the old drawable stretched into the new geometry for a frame;
`presentsWithTransaction`, raised for three presents starting with the adopting frame's (`PWT_FRAMES = 3`, drained one per frame from the frame that arms it;
set before the acquire —
the flag is captured there), folds new content and new bounds into one commit.
Costs a main-thread wait per present, so it is not left on.
- **Patch 11** (1 line, `src/window.rs`): keep the last pointer position when
the pointer leaves.
A press and a release are the two baseview events that carry no position of their own, so both are sent at `pointer_pos_in_points` —
and both are DROPPED when it is `None`, which `CursorLeft` set.
The release of a drag let go outside the window was therefore thrown away here, and egui, which deliberately keeps a drag alive across `PointerGone`, believed the button was held forever:
every `ScrollArea` in the editor refuses the wheel while anything at all is being dragged (`end_stranded_drag` in `harmonigraph-ui` is the shell-side backstop for it).
The position is the pointer's last known one, which is where a release lands;
`None` now means only that the pointer has never been seen, which is the case the guard is for.
- **Patch 12** (`src/renderer/{wgpu,opengl}/renderer.rs`, `src/window.rs`):
the same layer as Patch 10, across the other discontinuity —
a window coming back from occlusion.
Nothing can present while it is hidden, so the layer still holds the frame from before it went away and the compositor puts that back on screen the instant the window is visible again, before this process is told anything.
`Renderer::window_occluded` hides the layer while the window is occluded, so the re-expose composites the host's background instead of a picture that has stopped being true:
a black hole for that frame rather than a lie —
honesty, not invisibility, since one frame is the floor either way (see the baseview occlusion patch).
The layer comes back in `before_present`, from the frame holding a drawable it is about to present, under `presentsWithTransaction` (raised for as long as the layer is hidden) so the two land in one commit.
That gate is the load-bearing part:
a successful acquire proves both that the window is visible —
the acquire refuses an occluded one —
and that there is something to show, so no occlusion event has to be trusted to arrive for the layer to come back, and no frame unhides ahead of content.
A lost `Occluded(false)` (a view momentarily without a window, a host reparenting the plugin) would otherwise leave a permanently blank editor.
The GL renderer draws into the view's own context and takes the no-op.
- **Patch 13** (`src/renderer/wgpu/renderer.rs`): publish egui's managed font
texture into `CallbackResources` after this frame's texture deltas and before paint callbacks prepare.
Harmonigraph's glyph callbacks bind that GPU allocation directly;
cloning the `wgpu::Texture` handle does not copy its pixels.
This removes the 4.67–24.69 ms whole-atlas mirror upload paid whenever a zoom first reaches a new label size.
- **Patch 14** (`src/renderer.rs`, both renderers, `src/window.rs`): cap the
font atlas at 4096 on devices whose texture limit is larger.
egui fixes the atlas width at the reported limit and doubles its CPU image's height, so an 8192-wide atlas retained 64–128 MiB after walking the lattice's zoom sizes;
a full texture delta and the GPU texture temporarily multiplied that cost into roughly 400 MiB of process memory.
The cap is still eight times the largest glyph Harmonigraph allows, and puts a 64 MiB ceiling on egui's CPU image before its 80% rebuild clears historical sizes.
At the fresh Name size, a full-range zoom reaches 32 MiB on its first sweep and stays there through every return trip;
the largest valid Name size reaches the 64 MiB ceiling once and reuses it too.
- **Patch 15** (`src/renderer/wgpu/renderer.rs`, re-exports): let an editor keep a `SharedGpuContext` across window lifetimes.
Only the instance, adapter, device and queue survive;
each open still creates a surface and an egui renderer with a fresh texture namespace.
The original adapter is checked against the new surface before reuse.
Device-loss callbacks mark a retained setup unusable, and a nonblocking poll at open delivers any pending loss before deciding to reuse it.
Failed renderer creation also clears the retained setup, so a later open can recover with a fresh device.
Creating a fresh instance keeps the previous `new_without_display_handle` descriptor.
Harmonigraph pairs this with its own cache of immutable lattice pipelines, keyed on instance, device identity and output format, so a closed editor does not depend on the driver's compiler cache still being warm when it reopens.
Measured on Metal:
constructing the lattice resources took 2.58 s cold and 49 ms warm;
reusing their compiled handles took 1.4 µs in the headless reopen test.
The first open of a newly loaded instance still compiles, and hot-reload builds retain their existing rebuild behavior.
- **Patch 16** (`src/translate.rs`, `src/window.rs`): take every modifier from the event that carries them.
Mouse events wrote only Alt, Shift and `command` (from Ctrl, even on macOS), and Cmd and Ctrl were otherwise toggled per modifier key, which reaches the view only while it has the keyboard.
A Cmd or Ctrl let go after Cmd-Tab, Cmd-Space, Ctrl-arrow or a Cmd-click on the host has its key-up delivered elsewhere, so `mac_cmd` or `ctrl` stayed held until the next whole press inside the editor.
egui reads either as the zoom modifier on a wheel, so every wheel became a zoom:
every settings `ScrollArea` stopped, while the pictures, which also read `zoom_delta`, went on zooming.
That is the symptom of #501's stranded drag with no drag at all, so `end_stranded_drag` had nothing to end and the Console stayed silent.
Mouse and key events now both replace all five fields with the event's own set, Cmd as `mac_cmd` and `command` on macOS;
upstream egui-baseview 0.7 already has this shape.
- **Patch 17** (`src/window.rs`): stop drawing frames for a surface that
keeps refusing to present (#989).
A present refused three times in a row (`REFUSALS_BEFORE_BACKOFF`) is offered another only every 250 ms (`SURFACE_RETRY`) instead of every timer tick, because an occluded window's acquire is refused rather than waited on, and drawing anyway submitted 60-144 unpresented frames a second against 13-37 ms of GPU work with nothing pacing them —
wgpu reclaims a frame's staged uploads only when its submission completes, so outrunning the device grew the in-flight set into gigabytes within a minute of switching away from the host.
Keyed on the refusals rather than the occlusion event, since wgpu-hal and baseview read occlusion off different windows and can disagree, and the first present that lands (or an `Occluded(false)`) clears the backoff.
Texture deltas stay exempt and are checked first, so a frame carrying one is never skipped and glyph coordinates never point into a stale atlas.
The same change also runs `free_textures` on all three render exits instead of only past the acquire:
a `free`-only delta on a bailing frame used to drop egui's texture-manager ids permanently, leaking the retired atlas exactly when the backoff above forces a frame through on the strength of consuming that delta.
`wants_render` and `after_render` are pure functions with their own tests in `vendor/egui-baseview`, which `ci.sh` runs.
- **Patch 18** (`src/window.rs`, `src/translate.rs`): reserve app shortcuts from the host (#1433).
`KeyCapture::CaptureShortcuts` captures its listed `egui::KeyboardShortcut`s even when no widget has keyboard focus, so the editor's appearance undo and redo (Cmd+Z, Cmd+Shift+Z, and Ctrl+Y off macOS) never reach the DAW while the editor window has the keyboard.
Every other key still passes to the host unless egui wants keyboard input.
Each mode's capture decision is now one `KeyCapture::status`, replacing the per-mode branches in `on_event` and the separate wants-keyboard check after them, with its own test.
`translate_virtual_key` lowercases a character before matching it,
so Shift and Caps Lock letters still produce egui key events.
- **Patch 19** (`src/window.rs` `wheel_delta`): flip a macOS wheel's x only while Shift is up (#1536).
Upstream negates x on every macOS wheel, but macOS has already turned a Shift+wheel into a horizontal delta carrying the vertical wheel's own sign, and egui folds a Shift wheel back to `x + y`,
so the flip reversed every Shift-wheel and with it the lattice's zoom.
Upstream egui-baseview 0.9.0 still flips unconditionally, so a migration carries this forward.
Unchecked on a trackpad: a sideways swipe with Shift held keeps its raw sign too, which is reversed if macOS swaps no axes for a precise scroll.
`wheel_delta` is a pure function with its own tests in `vendor/egui-baseview`, which `ci.sh` runs.
- **`[workspace]` table** (`Cargo.toml`): an empty `[workspace]` table, for the same ancestor-root-exclusion reason `vendor/baseview/Cargo.toml` carries one —
without it, a worktree under `.claude/worktrees/` walks past its own root to the main checkout's `exclude` entries, which resolve under a different prefix and exclude nothing here, and cargo refuses outright.
`ci.sh` runs `cargo test --manifest-path` against this crate from exactly there.
- **Patch 20** (`src/window.rs`, `src/renderer/wgpu/renderer.rs`, `src/frame_log.rs`, `Cargo.toml`): time frames against the display (#1551).
`tick_ms` and `render_ms` leave out the drawable wait, which `acquire_ms` reports on its own:
with three drawables, a frame the display paces waits most of a refresh for one in steady state, and a total counting that read as a frame that was always nearly too slow.
The `frame-log` feature (macOS, wgpu) writes every frame's timing to `~/Library/Logs/Harmonigraph/`:
the display link's timestamps and frame divisor from baseview patch 4, the frame's stages, when it submitted, the layer's `maximumDrawableCount`,
and, through the wgpu-hal present observer, the GPU's start and end on the present's command buffer and the drawable's `presentedTime`.
Together they classify a missed refresh as a late link callback, a wait for a drawable, late CPU or GPU work, or a late latch.
Built only on request (`--features frame-log` on the plugin);
`tools/frame-log-summary.py` reads the log.
- **Upgrade**: use the [published-cohort feasibility map](docs/gui-cohort-feasibility.md) before replacing this package.
The egui 0.36 publication trigger is met and investigated,
and egui-baseview 0.9.0 has been re-evaluated since (#1529);
the current recommendation is to retain the product stack until the need-driven trigger the map names.
Published egui-baseview 0.7.2 replaces the Queue API and some old patch topologies,
so do not mechanically reapply every numbered patch.
The map records retained obligations,
including pre-input host-size adoption,
beyond this numbered inventory,
and the toolchain,
wgpu-hal and Metal-corpus costs of a future migration.
- **Upstreaming**: declined (2026-07-19).
Kept as local patches rather than proposed upstream.

## Historical: nih-plug fork (retired)

Before migrating to nice-plug (which supports host→plugin window resizing natively via `ResizeHint`/`Editor::set_size`), this project carried a nih-plug fork implementing that feature:
`yan-h/nih-plug`, branch `host-window-resize`.
The branch is kept for reference but is no longer a dependency.
