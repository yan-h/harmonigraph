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

## 2026-11-03 (archive copy)

- Upload the full H.264 video; keep a small AV1 copy (SVT-AV1, preset 6, CRF 28) rather than the video once it is uploaded. **Archive copy** writes it beside each export, off fresh, and a recorded look takes it from the Video pane like the Encoder.
- Why AV1 from the finished video, measured on a 20 s slice of `take-2026-11-04_04-22-42` at 2560x1440@60: 6.3% of the video's bytes at VMAF 96.3 (worst frame 95.8) against it. x265 at CRF 20 matched that at 9.7%; 10-bit, preset 4 and `tune=0` bought nothing. Encoding it from the finished video copies the soundtrack and its alignment as they are.
- Why not upload the copy instead: YouTube names only H.264 for uploads, and a simulated YouTube re-encode lost 0.5-0.8 VMAF more from the copy than from the full video.

## 2026-11-09 (performance audit, #1670)

- The Archive copy stays a second pass after the video: time to the video matters more than time to both files. With the Hardware encoder, encoding the two side by side in one ffmpeg finished both in 55 s against 79.5 s (5,302 frames at 2560x1440@60), but held the video back from 32 s to 55 s, and the video is the file used first. #1670 Q1

## 2026-11-10 (maintainability audit, #1677)

- An export row lists each distinct warning the renderer printed, one per line, so a take's note-history gap no longer hides that the export ran through x264. A failed archive copy is one of those warnings rather than its own channel. #1677 area 4 Q1
- An export row does not name the encoder the renderer opened: with every warning shown, the note that a Hardware export ran through x264 already reaches it, and naming it would make a line on the renderer's stderr a production contract. #1677 area 4 Q2
- A take recorded to a loop's end honours the Tail like any other take. Its exemption dated from #1147, when the tail was a fixed 4 s; since #1465 Tail is a setting, 0 fresh, so the exemption only overrode a Tail you chose. #1677 area 9 Q1

## 2026-11-14 (premises audit, #1739)

Calls marked "delegated" were made by the audit session after Yan asked it to settle the decisions with an obvious answer; the reason is part of the line.

- Takes keep their pass routing on the audio thread for now, rather than recording on the display's clock and choosing passes at read time (#1739 F3′). Reshaped, it comes out about even in lines; the routing fix chain ended on 2026-09-27, and Tape has not been tried in Bitwig. Reopen if a take fails on a fixed capacity (`SEGMENTS`, `RECORD_PASSES`) or Tape's note join shows losses in use. Delegated, declined for now. #1739
