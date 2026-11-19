# Intent: Lattice glow and bloom

Yan's answers from drift audits of this area (the `audit-drift` skill).
One line per decision; the audit reads this before asking anything.

## 2026-09-29 (all-areas run, #1314)

- A note's Background glow follows the Opacity it is drawn at, as its ink and bloom do. #1315

## 2026-11-14 (premises audit, #1739)
- The lattice glow's colour is carried on the CPU per frame (a light table the strip pass reads), retiring `InkHistory`, its row allocator and incarnations, and the release-colour loss on strip growth (#665's scoping note). The halo hue then follows gradient, range and width drags at once; judged on contact sheets before it counts as parity. To `propose-rework` with F4′. #1739 F4′ (No contact sheets were made: #1761 ran without them, and #1759 Q1 accepted the change.)

## 2026-11-14 (F4′ rework, #1759)

- A held or fading halo takes a gradient, range, width or tuning change in the frame its note does; no record asked for the old ease toward it over the Glow times. #1759 Q1
- A held halo's colour arrives where its note says, live and in exports, rather than stalling short of it. #1759 Q2
- Halos still fading survive a big chord arriving. #1759 Q3

## 2026-11-14 (drift audit, #1784)

- Clear display history (the Video pane's clear) keeps a held note's halo lit where it is, and drops only released notes' lights; a held halo no longer goes dark and fades back up over the Glow attack. Yan, 2026-11-14. #1784 G1
- Every value a halo carries takes its own direction: its brightness and each octave's and mark's share of its colour rise on the Glow attack and fall on the release. So a note joining a lit node brings its colour in on the attack, a note leaving takes its colour out on the release, an octave handoff does both at once, and a lingering halo does not dip when a note joins it. Yan, 2026-11-14 (B′). #1784 G2
- A slice's Thickness no longer weighs its share of the halo's colour: a slice is weighed by its light over the band's width at rest, however Thickness draws it. Yan, 2026-11-14. #1784 G3

## 2026-11-18 (maintainability audit, #1861)

Yan took the recommendations made after the audit was checked against the code ("go with all recommendations", 2026-11-18).

- Off-pane halo owner selection moves into `harmonigraph-scene`, and the glow fade's step reads the scene's own view. Built with #1838 in PR #1882; see `lattice-notes.md`. #1861 R5
