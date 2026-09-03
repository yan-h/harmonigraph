# PR draft: macOS frame timer should use kCFRunLoopCommonModes

- **Repo**: https://github.com/RustAudio/baseview
- **Branch**: https://github.com/yan-h/baseview/tree/frame-timer-common-modes
(one commit on upstream master;
`cargo check` clean)
- **Create PR**: https://github.com/yan-h/baseview/pull/new/frame-timer-common-modes

## Title

macOS:
run the frame timer in kCFRunLoopCommonModes

## Body

`TimerHandle` registers its `CFRunLoopTimer` in `kCFRunLoopDefaultMode`.
Timers in the default mode stop firing while the run loop is in an event-tracking mode, which macOS enters for native mouse-tracking loops:
window live-resize, menu tracking, and some host-driven drags.
For plugin GUIs driven by this frame timer, the UI freezes for the entire duration of such interactions and only catches up on mouse release.

Registering in `kCFRunLoopCommonModes` (which includes the tracking modes) keeps frames flowing.
This is the standard Cocoa remedy for exactly this symptom.

Found while debugging intermittent freeze-until-mouse-release window resizing in a baseview-based plugin (egui) under Bitwig on macOS;
verified fixed there.
The change is two call sites in `src/wrappers/appkit/timer.rs` (add + remove must use the same mode).
