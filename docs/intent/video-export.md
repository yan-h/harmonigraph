# Intent: Video export

## 2026-10-02 (issue sweep)

- Automatic exports use the Aspect and Output size captured when armed; explicit Re-render applies the current appearance and dimensions. #1314 Video Q2
- Remove the manual --layout presets. The take or replacement appearance owns placement and proportion. #1314 Video Q3

## 2026-10-06

- Transport stop finishes a Bitwig audio export: with the editor closed, the video covers the whole arrangement range. At bar is for ending at a chosen bar, not the export trigger. #1314 Video Q1
- The video runs a few silent seconds past the range end. Read as the renderer's default 4 s fade after the last note (`tail_of_render`), not checked against the take's WAV length. Whether export takes keep that fade, or it becomes a setting, is undecided.

## 2026-10-07

- The fade past the range end is the **Tail** setting, 0 s fresh, so a fresh export ends with its range or its recorded audio (#1472). This settles the 2026-10-06 open question.
- Hardware (VideoToolbox) is the fresh Encoder, captured with the DAW look (#1475).

## 2026-11-03 (drift audit, #1635)

- With "Use each take's recorded appearance", each queued take keeps its recorded framing (aspect, lattice side, split) and Video history; only Output size, Tail and Encoder come from the Video pane. #1635 Video Q1, #1637
- Hardware stays the fresh Encoder only if its keyframes do not pop: measure the keyframe grey change on the #833 take by #1411's method, and if it is worse than x264 with `-tune grain`, give Hardware a longer GOP or make x264 fresh again. #1635 Video Q2, #1637
- Keep Clear display history beside Record take: it clears what you see while recording, so the screen matches the export, which always starts from an empty picture. #1635 Video Q3, #1637
- Outcome, measured 2026-11-03: Hardware's keyframe pop is no worse than x264's with `-tune grain` (0.225 against 0.255 grey levels of excess change at a keyframe, 720p60, `take-2026-10-21_14-31-37`, since #833's take is gone), so Hardware stays the fresh Encoder and neither GOP changes. #1635 Video Q2, #1637
