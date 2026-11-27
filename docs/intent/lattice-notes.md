# Intent: Lattice notes, layout and motion

Yan's answers from drift audits of this area (the `audit-drift` skill).
One line per decision; the audit reads this before asking anything.

## 2026-09-29 (all-areas run, #1314)

- A soft note's name jumping to full, and its cross vanishing, in one frame at key-up is a bug. #1316

- Restore hover explanations for Note fade, Ring threshold and Ring attack/release on their compact diagrams. #1314 Notes Q2

## 2026-11-10 (maintainability audit, #1677)

- A node is visible on its presence before Opacity, as its name already is: a held note Opacity draws at nothing is named and hoverable off the home sheet as on it, which retires #1333's edge case. #1677 area 6 Q1

## 2026-11-14 (premises audit, #1739)

Calls marked "delegated" were made by the audit session after Yan asked it to settle the decisions with an obvious answer; the reason is part of the line.

- Note fade's duration becomes a look setting beside its curve, not a host parameter; its automation goes. Delegated: one gesture should not be split between the host and the look, and #1395 keeps note styling out of automation. #1739 D4, #1744
- One note record: the roll is the tracker, under four stated rules (zero-length notes, retrigger without an off, gaps, visibility stamped at release). To `propose-rework` with F4′. #1739 F4′
- Note state stays keyed per lattice position, not by the tuning's comma quotient: on Yan's tuning a chord's 32 positions would become 5 keys, the win is mostly on 12-TET, and it would add a quotient key with deferred merging to `motion.rs`. Declined for now; reopen on heavy 12-TET play or a noticed pan-in re-attack. #1739 F4′ piece 3

## 2026-11-18 (maintainability audit, #1861)

Yan took the recommendations made after the audit was checked against the code ("go with all recommendations", 2026-11-18).

- A zero-length note (the roll's Rule 1) pulses on the lattice too, as the rule says and the Spiral and the note band already do. Fixed with #1838. #1861 Q5
- Off-pane halo owner selection moves into `harmonigraph-scene` beside the rules it restates, taking an ownership predicate since the glow fade lives in the UI crate, and the lattice motion step reads the scene's own view. Built with #1838 in PR #1882. #1861 R5

## 2026-11-20 (questions from the #1861 build wave, #1893)

- Every lattice note waits out its attack before it leaves, as the Spiral and the note band already do, so a 1 ms note pulses as a 0 ms one does. The melody and bass marks follow the same held set: a note shorter than the Fade keeps its end until its attack lands, and is marked when the Fade outlasts the Mark delay (answer A, Yan, 2026-11-20). Built in PR #1912. #1893 item 7

## 2026-11-27 (drift audit, #1994)

- The naming reach becomes constants (12 fifths, 20 thirds), its two saved fields retiring: every value ever saved loaded as those. A saved project opens once with their `persist dropped` line. Yan, 2026-11-27, taking the recommendation. #1994 LN1, #1990
- Yan never uses the audio ring (Audio width above 0), so a case only it reaches lies outside the use; the ring stays as a feature, and the `posed` set built for it (#1507) goes. Yan, 2026-11-27. #1994 LN2, #1990
- Yan's pieces pan the lattice more than 12 fifths or 20 thirds within one Span, so the naming reach is centred on the lattice's origin rather than the camera, and a pan never respells a note. Yan, 2026-11-27. #1994 RL2, #1991 (Superseded 2026-11-27 by the line below.)
- The naming reach stays centred on the camera, LN1's constants alone: centred on the lattice's origin it named far just notes after unrelated nodes near the origin within the half-cent match (measured in PR #1996: the node at 36 fifths, G♯5 on the lattice, named A♭10+14), so a name can respell once the camera moves more than 12 fifths or 20 thirds from its node. Yan, 2026-11-27, answer (a) in chat. #1994 RL2, #1991, PR #1996
- Clear label history stays offered only in the History label mode, though Frame notes reads the same history in every mode (Clear display history also empties it). Yan, 2026-11-27, taking the recommendation. PR #1996
