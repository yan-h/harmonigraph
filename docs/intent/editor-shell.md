# Intent: Editor shell, layout and saved state

Yan’s decisions recorded for future drift audits.

## 2026-10-02 (issue sweep)

- Retain Tab / Hide tab bars for the live editor, saved as an editor preference rather than appearance. Tab takes priority over focus traversal outside text edits and restores hidden fold controls. #1314 Editor Q1

## 2026-10-22 (frame pacing, #1551)

- Drive the macOS editor's frames from the view's display link, with a third drawable: smoothness over up to ~7 ms of latency. #1551 Q1
- The editor frame limit becomes a fraction of the display rate (Display rate / Half / Third / Quarter); a saved cap reopens as the nearest fraction. #1551 Q2
- The display link keeps running while the editor is idle; pausing it is a later option. #1551 Q3
- Require macOS 14 and delete the macOS frame-timer path, with no fallback. #1551 Q4
