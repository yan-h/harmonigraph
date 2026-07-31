# Carried patches against upstream dependencies

Two dependencies carry local patches, both wired in via `[patch.crates-io]`
in the workspace `Cargo.toml`. Keep this file current when bumping either.

## baseview — vendored at `vendor/baseview/`

- **Upstream base**: `baseview 0.1.4` from crates.io (the version
  egui-baseview 0.3 pins; wired in via `[patch.crates-io]` in the workspace
  `Cargo.toml`, which covers both our direct dependency and egui-baseview's).
- **Patch** (3 lines, `src/wrappers/appkit/timer.rs`): register the macOS
  frame timer in `kCFRunLoopCommonModes` instead of `kCFRunLoopDefaultMode`.
  Default-mode timers stop firing while the run loop is in an event-tracking
  mode (native drags/resizes, menu tracking), freezing plugin GUIs until
  mouse release.
- **Patch 2** (4 sites: `src/platform/macos/{window,view}.rs`,
  `src/wrappers/appkit/view.rs` + `view/implementation.rs`): own the mouse
  cursor properly. `set_mouse_cursor` used a one-shot `addCursorRect`,
  which AppKit discards on the next cursor-rect rebuild — after which the
  HOST window's rects win, so the plugin's cursor appeared to depend on
  whatever Bitwig had behind the window. Now the desired cursor is stored
  and re-asserted from a `resetCursorRects` override, with
  `invalidateCursorRectsForView` forcing a rebuild on change. Because
  cursor rects only activate once the window becomes KEY (first click), a
  `cursorUpdate:` override (delivered via the tracking area's
  CursorUpdate | ActiveInActiveApp options) plus direct application on
  change-while-hovered were added for the freshly opened window as well.
  KNOWN LIMITATION: in Bitwig the cursor still doesn't take until the
  plugin window is first clicked (the host's not-yet-key window appears
  not to deliver tracking/cursorUpdate events); accepted as cosmetic —
  everything is correct from the first click on.
- **Patch 3** (3 files: `src/event.rs`,
  `src/wrappers/appkit/notification_center.rs`,
  `src/platform/macos/view.rs`): new `WindowEvent::Occluded(bool)`
  (mirroring winit's event), emitted from an
  `NSWindowDidChangeOcclusionStateNotification` observer filtered to the
  view's own window. On re-expose the view is also marked
  `setNeedsDisplay` so AppKit commits a fresh frame over whatever stale
  snapshot the compositor kept while the window was hidden. Together with
  the egui-baseview patch below, this fixes the outdated ghost image that
  stayed on screen after tabbing away from the host and back, until the
  window was clicked. macOS only; other platforms never emit the event.
- **Patch 4** (`src/lib.rs`, `src/window_open_options.rs`, `src/window.rs`,
  `src/platform/macos/{view,window}.rs`, `Cargo.toml`): make the frame timer
  configurable and re-armable, and report the display's refresh rate. The
  interval was the hardcoded `0.015` at the `TimerHandle::new` call site, so
  the window could never run faster than ~67 Hz — invisible on a 60 Hz panel,
  a hard ceiling on a 120/144 Hz one. Now `WindowOpenOptions::frame_interval`
  sets it at open and `Window::set_frame_interval` re-arms it live (storing
  the new `TimerHandle` drops the old, whose `Drop` unregisters it, so the
  cadence is replaced rather than stacked). `Window::display_max_fps` exposes
  `NSScreen::maximumFramesPerSecond` — needs the `NSScreen` feature on
  `objc2-app-kit` — so a caller can pace to the actual panel instead of
  guessing, and re-reading it per frame means the window follows a drag to
  another monitor. Intervals are clamped to `MIN_FRAME_INTERVAL`
  ..=`MAX_FRAME_INTERVAL` so a bad value can neither spin the run loop nor
  stall the window. macOS only; `set_frame_interval` is a no-op elsewhere and
  `display_max_fps` returns `None`.
- **Upgrade**: download the new crates.io tarball into `vendor/baseview`,
  re-apply the `kCFRunLoop*` lines, the cursor-rect ownership patch, the
  occlusion-event patch, and the configurable frame timer.
- **Upstreaming**: good candidate; uncontroversial fix, helps every
  baseview-based plugin. baseview and nice-plug are both RustAudio projects,
  so the fix would land in exactly the stack this plugin uses.

## egui-baseview — vendored at `vendor/egui-baseview/`

- **Upstream base**: `egui-baseview 0.3.0` from crates.io (the RustAudio
  fork used by nice-plug).
- **Patch 1** (2 call sites, `src/window.rs`): after a `Queue::resize()`,
  the window resize triggered by the physical-size change passed physical
  pixels to `Window::resize()`, which takes logical points — on scaled
  displays the window/view ended up `pixels_per_point` times too large
  (2x-zoomed, bottom-left-anchored content on Retina). Convert with
  `points_per_pixel` at both call sites (build path and `on_frame`).
- **Patch 2** (1 site, `src/window.rs` `on_frame`): texture deltas (font
  atlas rebuilds after `set_fonts`, new images) are only uploaded inside
  `render()`, but frames with no repaint due skip `render()` and drop the
  deltas permanently — glyph coordinates then point into a stale atlas
  (scrambled text). Force a render whenever `textures_delta` is non-empty.
  Only visible for apps that throttle repaints; always-repaint apps mask
  it.
- **Patch 3** (`src/window.rs` + both renderers): stale-frame ("ghost")
  recovery after window occlusion. `Renderer::render` now returns whether
  a frame was actually presented; a skipped present (occluded window,
  outdated/lost surface — the wgpu renderer's early-return paths) no
  longer consumes the repaint request, so rendering retries next tick
  instead of freezing on the last presented frame. The new
  `baseview::WindowEvent::Occluded(false)` (see the baseview patch above)
  schedules an immediate repaint so the first frame after re-expose is
  fresh.
- **Patch 4** (2 sites, `src/renderer/wgpu/renderer.rs` `render`): flush
  staged uploads on the surface-not-available early returns. `render`
  uploads egui's per-frame vertex/index/texture data (via `update_buffers`
  /`update_texture`, i.e. `queue.write_*`) BEFORE it acquires the surface
  texture; those staging buffers live in wgpu's pending writes and are only
  reclaimed by a `submit()` (through wgpu-core `pre_submit`). The
  Occluded/Timeout and Suboptimal/Outdated/Lost arms returned WITHOUT
  submitting, stranding that frame's staging buffers. Combined with Patch 3
  (a skipped present retries every timer tick), a backgrounded plugin window
  re-ran `render` ~66×/s and accumulated staging buffers into the
  *gigabytes* within minutes — the memory dropped instantly on refocus,
  when the next presented frame's `submit` finally drained the backlog. Fix:
  each early return now submits `user_cmd_bufs` + the upload encoder (no
  drawable acquired, so nothing presents — the window stays frozen while
  hidden, which is expected — but pending writes drain and `maintain` runs
  every tick, so memory stays flat). Root-caused from a 26 GB balloon while
  tabbed away from Bitwig.
- **Patch 5** (1 site, `src/window.rs` `on_frame`): make delayed repaints
  actually come due. The repaint deadline was recomputed as
  `now + repaint_delay` on *every* tick that painted nothing, and egui
  rebuilds `repaint_delay` from scratch each pass (reset to `MAX` in
  `begin_pass_repaint_logic`, then the min of that pass's requests) while the
  UI closure runs on every tick, painting or not. So a steady
  `request_repaint_after(N)` re-based the deadline on each tick, and for any
  N longer than the tick interval (~15 ms, the macOS frame timer) `now` never
  caught up: the deadline receded forever and the window painted nothing
  until an input event or a texture upload forced it. Every delayed repaint
  was silently dead — the idle poll included, which went unnoticed because
  the plugin shell requests a repaint whenever it drains MIDI, and an idle
  window has nothing to show anyway. Fix: keep the EARLIEST pending deadline
  rather than overwriting it, and schedule the next one from the instant a
  frame actually painted instead of leaving it unset (clearing it to `None`
  cost a whole tick, so every capped interval ran one tick long). Found while
  adding the Panel pane's frame-rate cap, which is built on exactly this
  mechanism and did nothing at all without the fix.
- **Patch 6** (`src/window.rs`): plumb the frame timer through, so the app
  can pace its own window. `EguiWindowSettings::frame_interval` sets the
  opening cadence; `Queue::set_frame_interval` changes it from inside a
  frame (applied after the frame returns, where the `Window` is reachable —
  the same deferred shape `resize` already uses), and
  `Queue::display_max_fps` passes baseview's reading through.
  Re-arming from `on_frame` replaces the very timer whose callback is
  running; that is safe because the handle's `Drop` only unregisters it from
  the run loop and the closure is owned by the timer, not borrowed from the
  frame. This is what makes a frame-rate cap enforceable: egui's
  `request_repaint_after` cannot do it, since egui keeps the SMALLEST delay
  requested in a pass and any zero-delay `request_repaint` (input event,
  hover animation, a host draining MIDI) also forces the next pass to zero —
  so a cap built on it evaporates exactly when the UI is busy.
- **Patch 7** (2 lines, `src/renderer.rs` + `src/lib.rs`): re-export
  `WgpuSetup` alongside `GraphicsConfig`. `WgpuConfiguration` was already
  public, but its `wgpu_setup` field cannot be matched without the enum, so
  there was no way to reach the `device_descriptor` hook and request an extra
  device feature (we ask for timestamp queries, for the overlay's GPU-time
  row). Pure re-export; no behaviour change.
- **Patch 8** (`src/renderer/wgpu/renderer.rs`, `src/window.rs`): measure the
  frame's other two halves and hand them back through `Queue` —
  `tess_ms` (time in `egui::Context::tessellate`) and `egui_gpu_ms` (GPU time
  for egui's own render pass, via a timestamp query pair). Both sat in blind
  spots: tessellation runs after the update closure returns, so it is neither
  the app's own frame time nor GPU time, and a wgpu paint callback's timer
  brackets only ITS passes, never the 2D UI around them. A frame cost could
  live entirely in either and read as zero everywhere. Both samples are
  BEGINNING-of-pass writes for the reason `harmonigraph-render` found the hard way:
  Metal grants `TIMESTAMP_QUERY_INSIDE_ENCODERS` and end-of-pass writes, then
  records zero for both, silently. The closing sample therefore rides a 1x1
  no-op pass placed after egui's. Readback is a three-step cycle polled with
  `PollType::Poll`, never `Wait` — blocking for the number would stall the
  pipeline being measured.
- **Patch 9** (`src/renderer/wgpu/renderer.rs`, `src/window.rs`): split the
  upload reading, then fix what the split exposed. `last_ubuf_ms` times
  `update_buffers` on its own, because the surrounding `last_upload_ms` also
  spans creating the command encoder, taking the renderer's write lock, and
  the MSAA resize — none of which are uploads, and two of which are places a
  frame blocks rather than works.
  With those apart, the resize guard turned out to fire every frame. It tested
  `msaa_texture_view.is_none()`, but `RendererOptions::default()` sets
  `msaa_samples` to 0 and `resize_and_generate_msaa_view` only fills that view
  when it is above 1 — so the view was never `Some`, and every frame
  reconfigured the surface to the size it already had. Rebuilding a swapchain
  per frame cost 0.5-3 ms, wandered on a ~1 s period as GPU load moved, and
  was unattributable: it read as "buffer uploads" on the perf overlay while
  `update_buffers` itself measured 0.3 ms. The term is now gated on
  `msaa_samples > 1`; `width`/`height` start at 0, so the first frame still
  configures.
- **Patch 10** (`src/renderer/wgpu/renderer.rs` `fold_present` module +
  hooks, `Cargo.toml` objc2 deps): present a resize without the flash
  (issue #121's fold flicker). Two artifacts, one layer: wgpu presents into
  a `CAMetalLayer` that raw-window-metal adds as a plain (not view-backed)
  sublayer, synced to the view's root layer from a KVO observer — so every
  geometry change it takes carries CA's default quarter-second implicit
  ease (of the presentation layer only; the model reports final values,
  which is why #120's probes saw "nothing animating"), and it takes those
  changes inside the HOST's transaction, out of reach of any
  `CATransaction` wrapped around our own resize calls. A null `actions`
  dictionary on the layer kills the lookup for every caller. Second: a
  present is decoupled from the transaction carrying the new bounds, so
  the resize commit shows the old drawable stretched into the new geometry
  for a frame; `presentsWithTransaction`, raised for three presents
  starting with the adopting frame's (`PWT_FRAMES = 3`, drained one per
  frame from the frame that arms it; set before the acquire — the flag is
  captured there), folds new content and new bounds into one commit. Costs a
  main-thread wait per present, so it is not left on. Includes a bounded
  probe: 48 stderr lines per resize gesture of model-vs-presentation
  geometry for our metal layer AND the topmost ancestor layer (the host
  window's frame view — a host-side resize animation glides there, in
  model or presentation depending on who animates), arming only when pwt
  is down and no window is still draining, so a border drag logs one
  window, not one per step. The renderer's size starts at zero, so editor
  open counts as a gesture and logs one window too.
- **Upgrade**: download the new crates.io tarball into
  `vendor/egui-baseview`, re-apply the two conversions, the
  texture-delta forced render, the occlusion/skipped-present patch, the
  staged-upload flush, the repaint-deadline fix, the frame-timer
  plumbing, the `WgpuSetup` re-export, the tessellation/egui-GPU timers,
  the upload split with its per-frame-reconfigure fix, and the
  fold-present module with its hooks and objc2 deps.
- **Upstreaming**: clear-cut bug fix; affects their own `ResizableWindow`
  helper on any HiDPI display. PR to the RustAudio repo.

## Historical: nih-plug fork (retired)

Before migrating to nice-plug (which supports host→plugin window resizing
natively via `ResizeHint`/`Editor::set_size`), this project carried a
nih-plug fork implementing that feature: `yan-h/nih-plug`, branch
`host-window-resize`. The branch is kept for reference but is no longer a
dependency.
