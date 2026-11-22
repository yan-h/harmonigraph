# Intent: Host window, frame pacing and input

Yan’s decisions recorded for future drift audits.
The 2026-10-22 section moved here from `editor-shell.md` when the 2026-11-02 drift audit gave this area its own record (#1635).

## 2026-10-22 (frame pacing, #1551)

- Drive the macOS editor's frames from the view's display link. Prefer smoothness over latency: keep the default 3 drawables rather than 2. #1551 Q1
- The editor frame limit stays an fps number, applied as the display link's preferred frame rate; if a 72 fps cap does not hold on the 144 Hz LG, fall back to fractions of the display rate (Display rate / Half / Third / Quarter, every Nth refresh). #1551 Q2
- Outcome, measured 2026-10-22: the preferred frame rate rounds to divisors of the 144 Hz LG's rate (120 ran at 144, 90 and 60 at 72), so the fallback shipped as Display rate / Half / Third / Quarter in #1553. #1551 Q2
- The display link keeps running while the editor is idle; pausing it is a later option. #1551 Q3
- Require macOS 14 and delete the macOS frame-timer path, with no fallback. #1551 Q4
- Leave the remaining drawable wait: Bitwig's UI is unaffected with the editor open, so neither CAMetalDisplayLink nor skipping a render to leave the deep regime is built; reopen if the host feels heavier or hitches show at Display rate. #1551 step 5

## 2026-11-02 (drift audit, #1635)

- The performance overlay judges the frame rate against the rate chosen (display rate ÷ Editor frame rate divisor), not fixed 30 and 50 fps, so a divided rate running on target reads healthy. #1635 Host Q1, #1634

## 2026-11-09 (performance audit, #1670)

- The editor drops to its idle poll once nothing in view can still move, rather than drawing every refresh because the host streams silence: a still picture cost 280-320 ms per second of Bitwig's main thread at 144 Hz. #1670 Q2, #1673

## 2026-11-10 (maintainability audit, #1677)

- Each picture drawn this frame says whether it is still moving, from the state or stamp its own motion runs on, and a picture not drawn holds nothing, so a delay a picture gains reaches the repaint guard with no edit there. #1677 R1

## 2026-11-10 (host key routing audit, #1675)

- With a label selected and nothing focused, Cmd-X goes to the DAW alone and the label is left as it is: cut means nothing on a read-only label. Escape there is the editor's, and clears the selection. #1675 Q1, #1674

## 2026-11-10 (idle display ticks, #1676)

- While the editor idles, the display link skips the frames that would draw nothing: it ticks on the refresh the idle poll draws on, and any input or repaint request restores the chosen rate. This supersedes #1551 Q3 above ("the display link keeps running while the editor is idle"): the UI pass on every undrawn tick cost 107 ms per second of Bitwig's main thread. #1676 option 1, #1692
- Accepted with it: a note or audible audio arriving from a still picture can wait for the next idle tick, up to about 56 ms at 144 Hz, since the input worker has no wake. Yan judged it in Bitwig on 2026-11-10 and it felt fine; a wake from the input worker (#1676 option 2) is the fallback if it is ever noticed. #1676

## 2026-11-14 (premises audit, #1739)
- No committed Metal shader corpus: delete it, its provider, gates, workflow, skill and the wgpu-hal sites that served only it, and start the editor's loading job at plugin activation on the device the editor reuses. Branch builds Yan loads were built before the runner regenerated the corpus, so it only ever spared the first load after a merge or an OS update; up to about 2.4 s of loading screen there is accepted. Supersedes #1712. #1739 D2, #1746
- Idle pacing takes #1676 option 2 now: check whether a pass is due before running the UI, on the display link's clock, with a wake from the input worker and the 50 ms poll kept; this supersedes the 2026-11-10 option-1 line above and retires the prediction chain in the vendored crates and #1708. Chosen for maintenance, ahead of the recorded trigger. #1739 D3, #1747

## 2026-11-17 (the shared own window, yan-h/otonal#1673)

- The own window remembers only where it was: each instance opens its own window at the size its project saved, rather than at the last own window's size. yan-h/egui-plugin-window#12 (answer A)
- The own window has a maximum size, and keeps its top-left corner when a pane folds. yan-h/otonal#1673 (The value is the implementer's, derived in `MAX_SIZE`'s comment, not Yan's choice.)

## 2026-11-18 (maintainability audit, #1861)

Yan took the recommendations made after the audit was checked against the code ("go with all recommendations", 2026-11-18).

- The armed Record button's breath and a running export's progress stripes animate at about 30 Hz, rather than holding the editor at the display rate, since a full redraw at 144 Hz costs Bitwig's main thread (#1673); the cues stay. #1861 Q1
- The input worker owns the input state only it reads (the event and audio queues, the audio position and origin, the analysis, the Tape notes, the recorder handle) instead of keeping it under the editor lock; frames keep the clock, and the take's end debounce stays where it is. #1861 R1, built in #1887

## 2026-11-18 (#1814, in chat, after step 7)

- The host's window keeps the own window's limits too, maximum included, since the editor adopted `EditorSize` (#1814, recorded in `editor-shell.md`).

## 2026-11-21 (pixel-rate pacing, #1911)

- Each picture drawn says when it next changes on screen rather than whether it moves:
  every frame while a value fades, and the frame its scroll next crosses a device pixel for the live roll, the heatmap and a Tape playhead.
  The editor draws at the soonest, and a pace at or past the idle poll is left to the poll.
  Yan said yes in #1893 item 3.
  This replaces the R1 line's yes or no (2026-11-10) with a time, still answered by each picture's own draw, and replaces the Tape playhead's 144 px/s threshold (#1837).
- Learn's armed pulse animates at about 30 Hz, as the Record button's breath does, rather than at the display rate. Yan, in chat, #1911.
- The Record breath and the export stripes keep their 30 Hz cadence (#1861 Q1): every frame of a breath differs, so pacing by pixel would put it back at the display rate.
- The heatmap's tail is paced too: every frame while the display it reads moves or Stars twinkle, and otherwise at the sooner of its scroll's and its cloud drift's next device pixel, with the colour memory stepped at that pace.
  Yan accepted the visual change "as long as shader effects are not significantly affected" (2026-11-21, in chat, #1914): the drift keeps its motion, Stars keep every frame, and only the colour memory's ease is stepped, by about twice the 30-against-60 fps difference measured in `MemoryFrame::follow`.

## 2026-11-21 (egui 0.37 and wgpu 30, PR #1916, in chat)

- The editor's surface is colour-managed as sRGB, as wgpu 30 configures it by default, rather than left unmanaged as under wgpu 29, which showed its colours more saturated on a wide-gamut display.
  Yan kept it: the editor reads closer to the exported video (tagged BT.709). PR #1916
