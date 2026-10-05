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
