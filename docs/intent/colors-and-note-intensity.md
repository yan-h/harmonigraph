# Intent: Colors and MIDI note intensity

Yan's answers from drift audits of this area (the `audit-drift` skill).
One line per decision; the audit reads this before asking anything.

## 2026-09-29 (all-areas run, #1314)

- Fresh intensity (velocity and gain to Opacity) leaves velocity visible only on soft notes, because unity gain adds its whole weight. That is fine; Yan may retune the defaults later. Superseded by the 2026-11-03 line: fresh routes nothing.

## 2026-10-02 (issue sweep)

- Spiral note dots follow the Opacity mapping, including their bloom and shadows. Rim names retain their envelope-driven legibility. #1314 Colors Q2

## 2026-11-03 (drift audit, #1635)

- Fresh intensity routes nothing (full opacity, Thickness base 0.989), captured with the DAW look on 2026-10-07 (#1475); this supersedes the 2026-09-29 velocity-and-gain-to-Opacity line. #1635 Colors, #1642
