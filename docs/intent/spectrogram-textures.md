# Intent: Spectrogram textures

Yan’s decisions recorded for future drift audits.

## 2026-10-05

- Remove the spectrogram's level contours (Contour strength, Contour levels, Contour edge softness) entirely: Yan did not use them. This supersedes the two 2026-10-02 contour decisions from #1314 (fresh strength 0%, block disabled under full Stars).

## 2026-11-03 (drift audit, #1635)

- Watercolor's and Scales' drift, the stars' Drift direction and the lattice glow material's drift accumulate frame by frame, as Star speed does since #1624, so a speed or direction drag changes the pace without jumping the texture; exports start from the same offset `now × speed` gives today. Speed and direction leave the colour memory's key. #1635 Textures Q1, #1639
- Editing the Audio level colours, or switching a preset, eases the held colour over Color release like a Level or Tilt drag (#1578) rather than resetting colour memory. Edge pooling, its width and its softness still reset it: they move what sits under each texel, #1579's reason. #1635 Textures Q2, #1639
