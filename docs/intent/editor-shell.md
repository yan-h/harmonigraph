# Intent: Editor shell, layout and saved state

Yan’s decisions recorded for future drift audits.

## 2026-10-02 (issue sweep)

- Retain Tab / Hide tab bars for the live editor, saved as an editor preference rather than appearance. Tab takes priority over focus traversal outside text edits and restores hidden fold controls. #1314 Editor Q1

The display link and frame pacing decisions (#1551) are in `host-window-and-pacing.md`.

## 2026-11-02 (drift audit, #1635)

- One wheel or trackpad zoom of the Analyzer is one undoable look edit, as a drag is: the gesture closes as one entry when it stops. Navigation stays in the look, so slots keep carrying the Analyzer's pitch range. #1635 Editor Q1, #1633

## 2026-11-10 (maintainability audit, #1677)

- Tail and the Tuning page's delay and map-offset fields are value bars, so they reset on double-click and type on Cmd-click like every other number; Stop at bar, ranging to 100,000, stays a drag value, the one a clippy rule lets through. #1677 area 3 Q1
- A bar's hover ends with the gestures that bar answers to, supplied by the widget in one wording, so no hover hand-writes them and none promises a reset or typed entry its bar lacks. #1677 area 3 Q2
