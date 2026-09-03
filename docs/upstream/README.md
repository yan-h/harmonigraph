# Upstream contribution drafts

Fixes this project carries locally (see ../../PATCHES.md) that belong upstream.
Prepared branches live on yan-h's forks where porting was mechanical;
nothing has been submitted —
submitting is a human call.

| Fix | Upstream | Status |
| --- | --- | --- |
| Frame timer run-loop mode | github.com/RustAudio/baseview | Branch `frame-timer-common-modes` on yan-h/baseview (applies to master); PR text in `baseview-frame-timer.md` |
| Cursor-rect ownership (stable cursor over plugin) | github.com/RustAudio/baseview | Patch vs 0.1.4 in `baseview-fixes.patch` + draft in `baseview-cursor.md`; master moved cursor code to `platform/macos/context.rs`, so the port needs a human look |
| Queue::resize physical/logical units + dropped texture deltas | codeberg.org/RustAudio/egui-baseview | Both patches vs 0.3.0 in `egui-baseview-fixes.patch`; PR text in `egui-baseview-resize-units.md` (covers both) |
| Editor resize race + host-resize opt-in | codeberg.org/RustAudio/nice-plug | Issue/PR text in `nice-plug-egui-resize.md` (design discussion first) |
