# Intent: Tape mode

Yan’s decisions recorded for future drift audits.

## 2026-11-14 (drift audit, #1784)

- An audition keeps the Lattice Map selection and offset recorded at its spot, since Yan often automates the map over time, and draws and names under the live tuning axes, which are never automated (#1756). Resolves #1735. #1784 T1
- In Tape mode a press on the tape picture only cues, and pans nothing. Resolves #1738. #1784 T2
- Cut the starved timer, the shown-first order of a whole-tape re-analysis and the meter re-anchor past a short pass's end, and prime the stretcher with the recording before the spot as read, without its walk across gaps. #1784 T3
- 0.25× stays. Its test keeps loose tolerances, because the stretch crate exposes no seed for its phase randomisation. #1784 T4

## 2026-11-18 (maintainability audit, #1861)

Yan took the recommendations made after the audit was checked against the code ("go with all recommendations", 2026-11-18).

- The Tape heatmap shares the spectrogram's folded grid, keyed on a Tape revision count bumped when slabs settle; folding `SongAxis` into `TimeAxis` is optional; #1837 and #1842 are fixed in their own PRs. #1861 R4 (Built in #1892, with #1842's fix: the revision advances on every write to a copy, `put` and `clear`, since the copy the pane draws is written only by the sync and never settled. `SongAxis` was not folded into `TimeAxis`: it carries the stretch shown, `Window`, which the coverage band, the ruler, the playhead's speed and the heatmap read and `TimeAxis` has no place for.)
