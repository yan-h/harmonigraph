# Intent: Video export

## 2026-10-02 (issue sweep)

- Automatic exports use the Aspect and Output size captured when armed; explicit Re-render applies the current appearance and dimensions. #1314 Video Q2
- Remove the manual --layout presets. The take or replacement appearance owns placement and proportion. #1314 Video Q3

## 2026-10-06

- Transport stop finishes a Bitwig audio export: with the editor closed, the video covers the whole arrangement range. At bar is for ending at a chosen bar, not the export trigger. #1314 Video Q1
- The video runs a few silent seconds past the range end. Read as the renderer's default 4 s fade after the last note (`tail_of_render`), not checked against the take's WAV length. Whether export takes keep that fade, or it becomes a setting, is undecided.
