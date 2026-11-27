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

- Takes are recorded on the display's clock and the offline reader chooses passes and song time (take v7; earlier passes come out through `--pass N` rather than separate files): the reason takes were stamped in song time, lining up with a separate audio bounce, is gone (#903, #972, #1196, #1342). To `propose-rework` as F3′'s take half; the Tape half waits until Tape has been used in Bitwig. First declined on delegation as quiet; reopened after a Fable review and answered by Yan. #1739 F3′ (Tape half left 2026-11-23, below.)

## 2026-11-14 (F3′ take half, #1760)

- Passes are stored as one take file per arm, with one WAV per pass beside it, and the offline reader chooses the pass and its song time from the file: the full rework, so the closure code is rewritten. #1760 Q1
- A loss of note history that touched no recorded run does not mark a take incomplete: it warned about a hole the take did not have. #1760 Q2
- Takes recorded before take v7, the 2026-11-04 one included, are not converted: they render only with a renderer from before the cut (`c423bf44`). #1760 Q3
- Behaviour holds only at the end: the reader and the cut to v7 are one PR. #1760 Q4
- A failed take warns on every pass, an earlier whole one included: the `PassClosed` record that kept a pass closed before the failure whole is dropped, with the proof that wrote it. Gaps still mark only the passes their inputs touch. #1766

## 2026-11-14 (take v8, F3′ simplified; tracked in #1760 and #1761)

Yan approved a search's three relaxations; each supersedes part of the entry above.

- R1: a record is placed by when it sounds, not where its input arrived. It supersedes step 1′'s input-placement rule and the routing in #1760 Q1. A note or release within about one delay of a pause or the take's end is drawn where it sounds, or dropped when no audio was recorded there.
- R2: the writer decides when a take is closed, rather than a proof on the audio thread. A plugin destroyed mid-take fails the take. The frontier it first sealed behind, and its flood caveat, are superseded by Yan's answer on #1776: a stopped take seals when its run's end is written.
- Seeks record on (Yan, on #1776): neither a forward seek nor, under Transport stop, a backward jump ends a take; Loop end still ends at the wrap, and Transport stop ends after 0.4 s of stillness.
- R3: a take is one WAV with no passes. It supersedes `--pass N`: a looped take renders all its loops as one video, and `--start` picks out the last. #1766's failure-marks-every-pass rule becomes the plain rule that a failure marks the take.
- Takes v7 and older stop loading, with no converter.

## 2026-11-15 (merge audit, #1803)

- The take's end releases what is still held: the reader ends every voice held at the last run's end, there, as a session reset does, so a Tail fades the last chord and counts from the take's end. It adds to R1 above, whose drop of a release that sounds after the last recorded block left such a note held through the tail. A take that ends with nothing held is unchanged, and an export at Tail 0 still ends with its audio. #1803 option 1

## 2026-11-18 (maintainability audit, #1861)

Yan took the recommendations made after the audit was checked against the code ("go with all recommendations", 2026-11-18).

- The six ignored PNG probes in the offline renderer's frame tests and the closed #503 heatmap timing benchmark are deleted, left to git history. #1861 Q4

## 2026-11-20 (one Appearance row, PR #1902)

- "Use each take's recorded appearance" (above) is the Appearance row's Recorded option, beside Current and the slots; what it keeps from the take is unchanged. Yan ("Go with your recommendations") (Slot half superseded by the 2026-11-24 removal of look slots.)

## 2026-11-22 (take sample rate, #1928)

- The sample rate travels with the audio: a take's WAV takes its rate from the first audio it records, and a rate change after that fails the take on the status line, with no render; a take whose audio all follows a change is recorded at the new rate. The take header no longer carries a rate. Yan ("build the general fix for A"), #1928 A

## 2026-11-23 (Tape used in Bitwig, #1925)

- Tape has been used in Bitwig, the condition F3′'s Tape half and #1925 part 1 waited on, and both are left: Tape keeps its own note recorder, and the take and the Tape stay two recordings with their own placement rules. Yan's use showed no Tape notes sitting wrong, no wish to export what Tape holds without arming a take, and no need for Sidechain on Tape, its ~29-minute cap or a Tape that survives a reload, so either change would buy maintenance alone, and part 1 at a high risk of a silently wrong export. Yan, 2026-11-23 ("Leave both"), #1925 part 1, #1739 F3′

## 2026-11-23 (rework proposal, #1956)

- An export names each setting of the take's recorded look that this build repairs on load, as it names an unread key since #1932. #1956 Q2

## 2026-11-24 (issue sweep, #1964)

- An export's warning lines name no source: `warning: the look holds view.sevens_size at -1.0, which this build loads as 0.15`, since the Video pane passes its Recorded look as a replacement too; a refusal still names its source. Yan, 2026-11-24, option A, #1964 item 1

## 2026-11-25 (Frequency range: Fit piece, #1976)

- An export can fit the Spectral pane's pitch range to what the piece fills, since 20-50 Hz is usually empty in Yan's pieces: the Video pane's Frequency range row, Fit piece or Dialled. Yan, 2026-11-25, #1976
- Fit piece is the fresh choice, and a saved look or recorded take without the key loads as Fit, so re-exports of older takes fit too. Yan ("go with recommendations"), #1976
- A pitch counts when the volume colours draw it a fifth of the way up for half a second in all, and every recorded note counts, with a semitone of margin. Yan ("go with recommendations"), #1976 (Superseded 2026-11-27 by #1994 TP1, docs/intent/tape-mode.md: sources hidden with Show are left out.)
- Fit piece frames the whole 20 Hz-20 kHz axis whatever the Analyzer's Frequency range, as Fit video ignores History duration; Dialled is the way to crop by hand. Yan, 2026-11-25, #1976
- A hand-run `--start` or `--end` render fits to exactly the soundtrack its frames analyze, the read before the first frame and past the last included, and to the notes up to its end, from the take's start, since the first frame's roll draws notes released before it. Yan, 2026-11-25, #1976
- "Half a second in all" stays an absolute time rather than a share of the render's length, though leakage over the lit level adds up with length and may reopen 20-60 Hz past about four minutes; exports are normally a minute or two. Yan, 2026-11-25, #1976
- No one-shot Fit button on the Analyzer's Frequency range bar for now: it would make the preview match the export, but has to be pressed after playing the piece, and a take's look is recorded when armed. Yan ("go with recommendations"), #1976
