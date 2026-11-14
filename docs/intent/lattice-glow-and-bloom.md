# Intent: Lattice glow and bloom

Yan's answers from drift audits of this area (the `audit-drift` skill).
One line per decision; the audit reads this before asking anything.

## 2026-09-29 (all-areas run, #1314)

- A note's Background glow follows the Opacity it is drawn at, as its ink and bloom do. #1315

## 2026-11-14 (premises audit, #1739)
- The lattice glow's colour is carried on the CPU per frame (a light table the strip pass reads), retiring `InkHistory`, its row allocator and incarnations, and the release-colour loss on strip growth (#665's scoping note). The halo hue then follows gradient, range and width drags at once; judged on contact sheets before it counts as parity. To `propose-rework` with F4′. #1739 F4′
