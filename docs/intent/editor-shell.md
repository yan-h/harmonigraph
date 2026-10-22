# Intent: Editor shell, layout and saved state

Yan’s decisions recorded for future drift audits.

## 2026-10-02 (issue sweep)

- Retain Tab / Hide tab bars for the live editor, saved as an editor preference rather than appearance. Tab takes priority over focus traversal outside text edits and restores hidden fold controls. #1314 Editor Q1

## 2026-10-22 (frame pacing, #1551)

- Drive the macOS editor's frames from the view's display link. Prefer smoothness over latency: keep the default 3 drawables rather than 2. #1551 Q1
- The editor frame limit stays an fps number, applied as the display link's preferred frame rate; if a 72 fps cap does not hold on the 144 Hz LG, fall back to fractions of the display rate (Display rate / Half / Third / Quarter, every Nth refresh). #1551 Q2
- The display link keeps running while the editor is idle; pausing it is a later option. #1551 Q3
- Require macOS 14 and delete the macOS frame-timer path, with no fallback. #1551 Q4
