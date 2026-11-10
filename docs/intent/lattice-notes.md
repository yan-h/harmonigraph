# Intent: Lattice notes, layout and motion

Yan's answers from drift audits of this area (the `audit-drift` skill).
One line per decision; the audit reads this before asking anything.

## 2026-09-29 (all-areas run, #1314)

- A soft note's name jumping to full, and its cross vanishing, in one frame at key-up is a bug. #1316

- Restore hover explanations for Note fade, Ring threshold and Ring attack/release on their compact diagrams. #1314 Notes Q2

## 2026-11-10 (maintainability audit, #1677)

- A node is visible on its presence before Opacity, as its name already is: a held note Opacity draws at nothing is named and hoverable off the home sheet as on it, which retires #1333's edge case. #1677 area 6 Q1
