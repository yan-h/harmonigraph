# Intent: Stars texture

Yan's answers from drift audits of this area (the `audit-drift` skill).
One line per decision; the audit reads this before asking anything.

## 2026-09-29 (all-areas run, #1314)

- The fresh Stars look is intended as it now draws: the 09-26 capture under the later changes, with Star softness widening every depth and Medium quality.

## 2026-10-02 (issue sweep)

- Retain Uniform’s large-pane optimization; do not restart performance research as part of this sweep. #1314 Stars Q4
- Solo keeps every layer baking and updating colour history, hiding only composition. The full-field cost is accepted so toggles preserve the look. #1392

## 2026-10-05 (merge audit, #1444)

- #1442 replaced the High, Medium, Low and Uniform profiles with one Stars resolution slider (default 75%), superseding the 09-29 "Medium quality" and #1314 Stars Q4.
Every resolution draws stars in their own pass into one image, so Uniform's large-pane split has no separate switch to retain.

## 2026-11-02 (drift audit, #1635)

- Star layers n draws stored layers 1..n in order, so fewer layers drop the front-most and more bring it back. The even spacing over five stored layers lost both its reasons (per-depth images and halo tiers, #1442; depth curves, #1454). #1635 Stars Q1, #1632

## 2026-11-10 (maintainability audit, #1677)

- No further background texture is expected to be added or dropped, so the lattice and the spectrogram keep their own texture enums (`LatticeMaterial`, `CloudStyle`) and their own control stacks; one shared enum and control function (R5) is declined. #1677 area 7 Q1

## 2026-11-18 (#1828, in chat)

- The Stars solo leaves the saved look for the editor window's own state (option A): a slot switch and an undo leave it as set, and it goes when switched off or when the editor closes. No save, slot or export holds it. #1828 (Slot half superseded by the 2026-11-24 removal of look slots.)
That a restore into an open editor keeps it too, and that a toggle is no longer an undo entry, follow from the move; they are the implementer's reading, not Yan's answer.
- A solo flag on a depth `Star layers` leaves out counts for nothing and comes back, lit, with its depth, however the count changes: the panel no longer clears every flag when the count moves there. #1862 (recommendation taken)
- Only the working lattice and spectrogram panes draw the solo; the Video tab's preview draws none, as an export does, so what the preview frames is what the render produces. #1862 (recommendation taken)

## 2026-11-27 (drift audit, #1994)

- The lattice's Material row drops None; Material amount 0 is its off switch, as #1353 made the pattern's. A project saved with None opens at Watercolor with a Console line, and a take whose look names None refuses to export. Yan, 2026-11-27, taking the recommendation. #1994 TX2, #1993
