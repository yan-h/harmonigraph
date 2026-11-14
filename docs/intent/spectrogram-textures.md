# Intent: Spectrogram textures

Yan’s decisions recorded for future drift audits.

## 2026-10-05

- Remove the spectrogram's level contours (Contour strength, Contour levels, Contour edge softness) entirely: Yan did not use them. This supersedes the two 2026-10-02 contour decisions from #1314 (fresh strength 0%, block disabled under full Stars).

## 2026-11-03 (drift audit, #1635)

- Watercolor's and Scales' drift, the stars' Drift direction and the lattice glow material's drift accumulate frame by frame, as Star speed does since #1624, so a speed or direction drag changes the pace without jumping the texture; exports start from the same offset `now × speed` gives today. Speed and direction leave the colour memory's key. #1635 Textures Q1, #1639
- The lattice glow's Pattern speed and Breathing speed accumulate the same way, so neither jumps the light on a drag. #1647's review, #1651
- Breathing speed 0 freezes the breathing where it is; depth 0 turns it off. Saved speed-zero looks with nonzero depth now show a frozen per-node pattern. #1658, 2026-11-04
- Editing the Audio level colours, or switching a preset, eases the held colour over Color release like a Level or Tilt drag (#1578) rather than resetting colour memory. Edge pooling, its width and its softness still reset it: they move what sits under each texel, #1579's reason. #1635 Textures Q2, #1639

## 2026-11-10 (maintainability audit, #1677)

- No capability record per spectrogram style: the separate style tests stay, since no further style is expected (#1677 area 7 Q1). Reopen only when a style is added. #1677 R7

## 2026-11-14 (premises audit, #1739)
- The live spectrogram history goes to `propose-rework` as one shared slab pyramid on the absolute grid (F2′): about half the memory, the same picture in the dock and the preview, the per-surface aggregator gone. #1739 F2′
