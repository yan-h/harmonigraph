# PR draft: Queue::resize passes physical pixels to a logical-points API

- **Repo**: https://codeberg.org/RustAudio/egui-baseview (Codeberg — submit
  via web UI; `gh` doesn't apply)
- **Patch**: `egui-baseview-resize-units.patch` in this directory, against
  the crates.io 0.3.0 sources (applies to current main at the same sites).

## Title

Convert physical size to logical points before resizing the window after
Queue::resize

## Body

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
