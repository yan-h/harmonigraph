# PR draft: two window.rs fixes (resize units, dropped texture deltas)

- **Repo**: https://codeberg.org/RustAudio/egui-baseview (Codeberg — submit
  via web UI; `gh` doesn't apply)
- **Patch**: `egui-baseview-fixes.patch` in this directory, against the
  crates.io 0.3.0 sources (applies to current main at the same sites).
  Two independent fixes; split into two PRs if the maintainers prefer.

## Fix 1: Queue::resize passes physical pixels to a logical-points API

When user code calls `Queue::resize(PhySize)`, the window loop notices the
physical-size change and calls `Window::resize()` with those physical
pixel values — but `Window::resize()` takes **logical points**. On any
scaled display the window/view ends up `pixels_per_point` times too large.
On a 2x macOS display the visible symptom is dramatic: the view becomes
twice the window size, showing only the bottom-left quadrant, i.e.
2x-zoomed content anchored bottom-left, unrecoverable until the window is
recreated.

This affects the crate's own `ResizableWindow` helper (via nice-plug-egui)
on every HiDPI/Retina display; it's invisible at scale factor 1.0, which
is presumably why it shipped.

Fix: multiply by `points_per_pixel` at both call sites (the build path in
`new()` and the per-frame check in `on_frame()`). Verified in a nice-plug
plugin under Bitwig on a 2x display.


## Fix 2: texture deltas are dropped on frames that skip rendering

`on_frame` uploads egui's `textures_delta` only inside `render()`, but
frames with no repaint due skip `render()` and discard the whole
`full_output` — deltas included. A font atlas rebuild (after
`set_fonts`) is emitted exactly once; if that frame's delta is dropped,
egui's new glyph coordinates point into the stale atlas forever
(scrambled text). Apps that repaint every frame mask the bug; apps that
throttle repaints hit it readily.

Fix: force a render whenever `textures_delta` is non-empty. Verified by
switching fonts at runtime in a repaint-throttled plugin.
