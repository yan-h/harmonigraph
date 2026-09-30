# Intent: MIDI ribbons and their bloom

Yan's answers from drift audits of this area (the `audit-drift` skill).
One line per decision; the audit reads this before asking anything.

## 2026-09-30 (first trial)

- A ribbon's bloom follows the opacity it is drawn at, as the lattice's does. #1289
- One note's shadow never darkens under another note, at any opacity. Low priority. #1293
- Bloom halo width depends on device pixels, so display scale and export resolution change it. Worth thinking about; undecided. #1294
- Note-name thinning should become one comprehensive placement pass across pitches. Low priority. #1295
- The ribbon width minimum (1.5 pt, applied before Thickness) is worth revisiting. #1296
- Tiny gaps and tiny notes are edge cases worth thinking about: butted notes merging into one ribbon, short notes stretched to a floor. #1297
- The rest of that trial's behaviour description: nothing obviously wrong. Don't re-ask item by item.
