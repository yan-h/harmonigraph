# Intent: Colors and MIDI note intensity

Yan's answers from drift audits of this area (the `audit-drift` skill).
One line per decision; the audit reads this before asking anything.

## 2026-09-29 (all-areas run, #1314)

- Fresh intensity (velocity and gain to Opacity) leaves velocity visible only on soft notes, because unity gain adds its whole weight. That is fine; Yan may retune the defaults later. Superseded by the 2026-11-03 line: fresh routes nothing.

## 2026-10-02 (issue sweep)

- Spiral note dots follow the Opacity mapping, including their bloom and shadows. Rim names retain their envelope-driven legibility. #1314 Colors Q2

## 2026-11-03 (drift audit, #1635)

- Fresh intensity routes nothing (full opacity, Thickness base 0.989), captured with the DAW look on 2026-10-07 (#1475); this supersedes the 2026-09-29 velocity-and-gain-to-Opacity line. #1635 Colors, #1642

## 2026-11-14 (premises audit, #1739)

Calls marked "delegated" were made by the audit session after Yan asked it to settle the decisions with an obvious answer; the reason is part of the line.

- The pitch colour range (Darkest and Brightest pitch) becomes a look setting beside the MIDI gradient, not a host parameter; its automation goes. Delegated: #1395 keeps colours out of automation. #1739 D4, #1744
- Opacity stays on a note's presence, keeping the second presence level: applying it to the ink alone would retire three code sites (the extra level's last field bug was fixed on 2026-10-01) at the cost of a look change nobody asked for, an opaque near-black band for every soft note. Removing the route outright is not an option: Yan's template carries velocity and gain to Opacity into most projects. Delegated, declined; Yan confirmed it on 2026-11-14 after contact sheets showed it changes little: under the template's routes no note falls below about 87% Opacity, and with a strong velocity route a soft note's ring becomes a grey outline about 2.7× brighter that fades out at the same time, with a barely darker shadow. #1739 D6
