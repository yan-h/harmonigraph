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
- What that leaves, the UI pass on every undrawn display tick (107 ms per second), is undecided, and one of its options supersedes #1551 Q3 above. #1676
