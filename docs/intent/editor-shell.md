# Intent: Editor shell, layout and saved state

Yan’s decisions recorded for future drift audits.

## 2026-10-02 (issue sweep)

- Retain Tab / Hide tab bars for the live editor, saved as an editor preference rather than appearance. Tab takes priority over focus traversal outside text edits and restores hidden fold controls. #1314 Editor Q1

The display link and frame pacing decisions (#1551) are in `host-window-and-pacing.md`.

## 2026-11-02 (drift audit, #1635)

- One wheel or trackpad zoom of the Analyzer is one undoable look edit, as a drag is: the gesture closes as one entry when it stops. Navigation stays in the look, so slots keep carrying the Analyzer's pitch range. #1635 Editor Q1, #1633

## 2026-11-10 (maintainability audit, #1677)

- The tempered links (Meantone, Marvel) are tuning, not appearance: the runtime holds them, resolved from the configuration's owner, and the plugin saves them in its musical settings. The standalone app, with no such settings, no longer keeps its links across a restart and opens with both linked. #1677 area 2 Q1
- One list of the appearance fields the host owns, `AppearanceDocument::copy_host_owned` beside `set_camera_movement`: a look resets them on capture and keeps the live document's on apply, and the state extractor's overlay is tested against it. #1677 R3
- One call both takes an app shortcut and claims it from the host, so a shortcut is registered in one place each frame. #1675 Q2, built in #1690
