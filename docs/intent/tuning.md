# Intent: Tuning

## 2026-10-02 (issue sweep)

- Pass through allows Learn to update lattice axes even when a live source has Retune enabled. Only active retuning reserves the axes as its target. #1314 Tuning Q1

## 2026-11-03 (drift audit, #1635)

- Show Adaptive's Context settings (Half-life, Same-note tolerance, Silence reset and the two Reset checkboxes) under Lattice Map while Follow harmony is on, since Follow reads them. #1635 Tuning Q1, #1638
- A Tune keeps its Tuning delay and reports it as latency under every engine, Pass through included, so switching engines never makes the host reactivate Tunes; a Tune added only to show a track's notes pays that buffer. #1635 Tuning Q2

## 2026-11-03 (Lattice Map settings)

- Drop the Lattice Map pane's assignment and sounding-interval lists. Hovering a key's assigned node shows its full correction on the lattice, and the lattice's held notes and comma-spelled names serve #889's "inspect actual intervals between sounding nodes"; the list only measured from the lowest voice. #889
- Lattice Map's controls are one section of blocks with no folds, its map renamed in a field beside the selector and deleted only through a confirming submenu.
