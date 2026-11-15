# Intent: Spiral

## 2026-10-02 (issue sweep)

- The current dot sizes are right. Keep their sizing curve and clarify the body clearance, removing the obsolete black-border rationale without changing the picture. #1314 Spiral Q1

## 2026-11-03 (drift audit, #1635)

- The Spiral's dots share the roll's surround, its 15 pt cut reach included, accepting that a cut dims part of the octave above and below on the note's own ray. #1635 Spiral Q1
- The Spiral's fit reserves no room past the track: drop `VOICE_OVERHANG`, whose radial tick #375 replaced with a dot clamped to half the track. #1635 Spiral Q2, #1641

## 2026-11-14 (premises audit, #1739)
- The Spiral's dots draw through the roll renderer and `glow.rs` goes: its bloom takes the ribbons' Coverage threshold in half floats (a fading dot's halo lasts as long as the dot), the dot cut stays on `dot_surround` (#1545), and an analytic holdout stops a translucent dot showing a neighbour's cut. #1739 D9, #1748
