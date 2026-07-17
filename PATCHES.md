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
  change-while-hovered cover the freshly opened, not-yet-clicked window.
- **Upgrade**: download the new crates.io tarball into `vendor/baseview`,
  re-apply the `kCFRunLoop*` lines and the cursor-rect ownership patch.
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
- **Upgrade**: download the new crates.io tarball into
  `vendor/egui-baseview`, re-apply the two conversions.
- **Upstreaming**: clear-cut bug fix; affects their own `ResizableWindow`
  helper on any HiDPI display. PR to the RustAudio repo.

## Historical: nih-plug fork (retired)

Before migrating to nice-plug (which supports host→plugin window resizing
natively via `ResizeHint`/`Editor::set_size`), this project carried a
nih-plug fork implementing that feature: `yan-h/nih-plug`, branch
`host-window-resize`. The branch is kept for reference but is no longer a
dependency.
