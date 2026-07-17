# PR draft: own the mouse cursor via resetCursorRects (macOS)

- **Repo**: https://github.com/RustAudio/baseview
- **Patch**: the cursor hunks of `baseview-fixes.patch` (against crates.io
  0.1.4; current master moved cursor code into
  `src/platform/macos/context.rs`, so the mechanical port needs adapting).

## Title

macOS: assert the mouse cursor from resetCursorRects / cursorUpdate

## Body

`set_mouse_cursor` adds a one-shot cursor rect. AppKit discards cursor
rects added outside a `resetCursorRects` override on the next rebuild,
after which the HOST window's rects win — in a DAW, the plugin's cursor
then visibly depends on whatever the host has BEHIND the plugin window.

Fix: remember the desired cursor in shared state; re-assert it from a
`resetCursorRects` override (the sanctioned mechanism); call
`invalidateCursorRectsForView` on change so updates apply immediately;
and also override `cursorUpdate:` (the tracking area already registers
CursorUpdate | ActiveInActiveApp) plus apply directly on
change-while-hovered, covering windows that are not yet key.

Found and verified in Bitwig on macOS. Known residual: in Bitwig the
cursor still doesn't take until the plugin window's first click (the
not-yet-key window appears not to deliver tracking events at all);
maintainers may know the missing trick.
