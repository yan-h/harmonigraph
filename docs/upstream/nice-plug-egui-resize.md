# Issue/PR draft: nice-plug-egui resize correctness

- **Repo**: https://codeberg.org/RustAudio/nice-plug
- **Status**: text only — both items touch API/UX design, so a discussion
  issue is the right opener rather than a cold PR.

## Item 1: request_resize race (peek vs consume)

`EguiEditor`'s update loop does:

```rust
if let Some(new_size) = egui_state.requested_size.swap(None) {
    if context.request_resize() { ... }
}
```

The host reads `Editor::size()` *during* `request_resize()`, and `size()`
returns `requested_size` only if still set — but it was already swapped to
`None`, so the host resizes the parent window to the *previous* size while
the plugin applies the new one. During a continuous drag this is a
one-frame lag; on the final movement the last delta never reaches the
host, leaving parent and child mismatched (on macOS: content shifted
toward the bottom, since AppKit anchors child views bottom-left).

Fix: peek (`load()`) before `request_resize()`, clear after the round
trip. Verified in Bitwig/macOS with a custom editor using this pattern.

## Item 2: opt-in host->plugin resizing for the egui adapter

nice-plug-core already supports host-initiated resizing
(`Editor::resize_hint`/`set_size`), but `nice-plug-egui`'s `EguiEditor`
doesn't implement either, so egui plugins never get a native resize border
even in hosts that support it (Bitwig does, for CLAP and VST3).

Proposal: an `EguiSettings::resize_hint: ResizeHint` (default
non-resizable, preserving current behavior). When resizable, `EguiEditor`
implements `resize_hint()`/`set_size()`: report the new size immediately,
apply it to the render surface and child view on the next frame — no
`request_resize` round-trip, since the host already resized the parent.
A working implementation of exactly this lives in the harmonigraph
editor (`crates/harmonigraph-plugin/src/editor.rs`, `set_size` +
`host_resized` handling); happy to adapt it into a PR if the shape sounds
right.
