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
- **Upgrade**: download the new crates.io tarball into `vendor/baseview`,
  re-apply the `kCFRunLoop*` lines.
- **Upstreaming**: good candidate; uncontroversial fix, helps every
  baseview-based plugin. baseview and nice-plug are both RustAudio projects,
  so the fix would land in exactly the stack this plugin uses.

## egui-baseview — vendored at `vendor/egui-baseview/`

- **Upstream base**: `egui-baseview 0.3.0` from crates.io (the RustAudio
  fork used by nice-plug).
- **Patch** (2 call sites, `src/window.rs`): after a `Queue::resize()`, the
  window resize triggered by the physical-size change passed physical
  pixels to `Window::resize()`, which takes logical points — on scaled
  displays the window/view ended up `pixels_per_point` times too large
  (2x-zoomed, bottom-left-anchored content on Retina). Convert with
  `points_per_pixel` at both call sites (build path and `on_frame`).
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
