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

## 2026-10-02 (issue sweep decisions)

These settle the open questions from the first trial above.

- Bloom radius follows the owning pane's shorter dimension, independently of render scale, clipping, the spectrum/history divider and Lead. The previous nine-tap profile is the reference at a 1080-pixel shorter pane edge. #1294
- Artistic lattice edge softness is one logical point; actual antialiasing retains a one-fragment floor. Analyzer backdrop stripes are one logical point wide, with Gap expressed in points. #1294
- Exports use a 1280-point-wide composition at every output resolution; previews scale its point-sized details with the composition. #1294
- One note-name collision pass measures actual letters and marks across pitches. Held notes win, then previously visible labels, then onset, pitch and voice break ties. Loudness does not reorder labels. A displaced visible label stays suppressed until it leaves the viewport; a view adjustment releases suppressions while retaining visible priority, and backward time clears history. #1295
- Positive Thickness receives the 1.5-point ribbon width minimum after mapping. Exact zero removes body and shadow. #1296
- A touching re-strike gets a short narrowing at its actual onset. This local notch may fall below the ordinary width minimum. #1297
- The minimum note length is one logical point, preserving the Retina reference. A shorter note extends flat at its onset pitch into the past; actual bends keep their timestamps and slopes. Lower-resolution output may resolve that minimum more softly. #1297
