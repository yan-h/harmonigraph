# Intent: Tape mode

Yan’s decisions recorded for future drift audits.

## 2026-11-14 (drift audit, #1784)

- An audition keeps the Lattice Map selection and offset recorded at its spot, since Yan often automates the map over time, and draws and names under the live tuning axes, which are never automated (#1756). Resolves #1735. #1784 T1
- In Tape mode a press on the tape picture only cues, and pans nothing. Resolves #1738. #1784 T2
- Cut the starved timer, the shown-first order of a whole-tape re-analysis and the meter re-anchor past a short pass's end, and prime the stretcher with the recording before the spot as read, without its walk across gaps. #1784 T3
- 0.25× stays. Its test keeps loose tolerances, because the stretch crate exposes no seed for its phase randomisation. #1784 T4
