# Intent: Tuning

## 2026-10-02 (issue sweep)

- Pass through allows Learn to update lattice axes even when a live source has Retune enabled. Only active retuning reserves the axes as its target. #1314 Tuning Q1

## 2026-11-03 (drift audit, #1635)

- Show Adaptive's Context settings (Half-life, Same-note tolerance, Silence reset and the two Reset checkboxes) under Lattice Map while Follow harmony is on, since Follow reads them. #1635 Tuning Q1, #1638
- A Tune keeps its Tuning delay and reports it as latency under every engine, Pass through included, so switching engines never makes the host reactivate Tunes; a Tune added only to show a track's notes pays that buffer. #1635 Tuning Q2

## 2026-11-03 (Lattice Map settings)

- Drop the Lattice Map pane's assignment and sounding-interval lists. Hovering a key's assigned node shows its full correction on the lattice, and the lattice's held notes and comma-spelled names serve #889's "inspect actual intervals between sounding nodes"; the list only measured from the lowest voice. #889
- Lattice Map's controls are one section of blocks with no folds, its map renamed in a field beside the selector and deleted only through a confirming submenu.

## 2026-11-10 (maintainability audit, #1677)

- Retire the JavaScript adaptive-tuning model, its examples and its fixture exporter: a policy change is no longer designed there first. The Rust policy, with its committed fixtures in `crates/harmonigraph-core/src/policy/fixtures.txt`, is the reference. #1677 area 1 Q1
- Restored host parameters are clamped by hand in `filter_state`, not in the vendored wrapper's restore loop: a patch there would have to be carried across every wrapper upgrade to replace nine arms. Declined. #1677 R11

## 2026-11-14 (premises audit, #1739)

Calls marked "delegated" were made by the audit session after Yan asked it to settle the decisions with an obvious answer; the reason is part of the line.

- The five tuning axes stay host parameters behind the owned-parameter mailbox. Making them plugin settings would delete about 1,200 lines but drop their automation, modulation and controller mapping, which `adaptive-tuning.md` names as a requirement ("restoring state or automating tuning works with the editor never opened"); no defect argues for it. Delegated, declined. #1739 D5
- One sample-indexed configuration timeline for every engine; `Owner.block` goes, Learn rides the next adopt, and a lost history falls back to the reducer's configuration under Adaptive. #1739, #1749
- Check the policy fixtures' context assembly against production's before trusting them as the reference; if a fixture line moves, the context moves into core. #1739, #1750
- Learn reads the Hub's rows and `ConfirmedPitches` goes, dropping its artificial 256-voice refusal. #1739, #1751
