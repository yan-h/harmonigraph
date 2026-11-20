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
- Restored host parameters are clamped by hand in `filter_state`, not in the vendored wrapper's restore loop: a patch there would have to be carried across every wrapper upgrade to replace nine arms. Declined. #1677 R11 Superseded by the 2026-11-18 line.

## 2026-11-14 (premises audit, #1739)

Calls marked "delegated" were made by the audit session after Yan asked it to settle the decisions with an obvious answer; the reason is part of the line.

- The five tuning axes become plugin-owned settings saved in the musical settings, not host parameters: Yan never automates, modulates or maps them, so the owned-parameter mailbox that exists only to make them automatable goes. First declined on delegation by citing `adaptive-tuning.md`'s requirement; reopened after a Fable review noted the requirement's need was not tested, and answered by Yan. #1739 D5, #1756
- One sample-indexed configuration timeline for every engine; `Owner.block` goes, Learn rides the next adopt, and a lost history falls back to the reducer's configuration under Adaptive. #1739, #1749
- Check the policy fixtures' context assembly against production's before trusting them as the reference; if a fixture line moves, the context moves into core. #1739, #1750
- Learn reads the Hub's rows and `ConfirmedPitches` goes, dropping its artificial 256-voice refusal. #1739, #1751

## 2026-11-14 (review of #1792)

- One lost-history path for every engine: where the configuration history no longer reaches an attack's sample, Lattice Map assigns from the live map and configuration as Adaptive does, rather than sounding the note raw and raising the policy fault. A Lattice Map note struck just before a steady-clock discontinuity is tuned by the current map. Yan's call. #1792

## 2026-11-14 (#1756)

- Bitwig's own undo no longer covers edits of the tuning axes, now that they are plugin settings rather than host parameters; no editor undo replaces it. Accepted by Yan. #1756, #1795
- A project saved while the axes were host parameters opens at fresh axes, keeping its links and policy, with one Console line saying so; there is no compatibility shim. Accepted by Yan. #1756, #1795
- After lost host input, editor tuning edits and Learn keep applying until the host resets; the fault notice says so, and a take recorded after the loss still refuses its configuration. Accepted by Yan. #1795

## 2026-11-15 (drift audit, #1798)

- Harmonigraph's own input runs at zero delay: its correction comes back within the same callback, so the one buffer it held its own track's MIDI, which the host never compensated, goes. #1798 H1, #1797
- A note whose correction came back too late is drawn and recorded at the raw pitch it sounded at, not the correction the Hub chose, so a take and its export match the soundtrack. #1798 H2
- The missed-corrections count and its red name last until Reset all voices or a delay change rather than clearing at every Stop; the other fault notices still clear at every cut. #1798 H3, #1797
- Tuning Delay becomes a saved Tune setting rather than a host parameter, as #1756 did for the axes; it leaves Bitwig's parameter list and existing Tunes reopen at 1×. #1798 H4, #1797
- Cut, as cases only review supposed: the Reset generation and its pause hook (Reset comes only from Harmonigraph's own row), Learn's "held state is incomplete" state, rows taking turns past 2,048 records in one callback, and the count of Note-Offs a stacked cut could not queue. #1798 H5, #1797

## 2026-11-15 (feature-scale pass, #1798)

- Yan no longer uses Adaptive but keeps it in the product; its cases lie outside the stated use, so it gets no new mechanism for its own rare cases.
- Live note-history loss is handled only by the take's gap record and its export warning: the live banner, per-source certainty and the withdrawn release fade go, since the Hub repays the snapshots within about a callback once the display lane drains. #1798 F1, #1797
- The Tuning pane sends the whole policy it shows rather than only the fields changed; a pane edit within one block of a keyboard Learn result may revert it until Learn runs again. #1798 F2, #1797

## 2026-11-18 (CLAP-shell rework, #1814)

- Restored host parameters follow one rule per row in harmonigraph-clap's parameter table: missing, wrong-typed or an unknown variant takes the default, and out of range is clamped. This supersedes the 2026-11-10 `filter_state` line: its reason, a patch carried across every wrapper upgrade, ended when harmonigraph took the wrapper as its own crate (#1827). #1814 step 5

## 2026-11-18 (maintainability audit, #1861)

Yan took the recommendations made after the audit was checked against the code ("go with all recommendations", 2026-11-18).

- "Waiting for host activation" keeps the fault colour: it is a status to inspect, shown only before the first activation. The status-string check (#1841) is fixed without changing it. #1861
- One CLAP test host serves all five suites that now carry their own, in `harmonigraph-clap` behind its test feature, with Retune or Adaptive chosen explicitly; the take suites move out of `configuration/tests.rs`. To be built after #1814 step 7. #1861 R3

## 2026-11-19 (#1881, PR #1884)

- With the CLAP shell running the steady clock on across a host's restart of its counter, a backward step no longer reaches the plugin, so a Lattice Map note struck just before a restart is tuned by the map in force at its own sample. This supersedes the 2026-11-14 #1792 line's sentence about a note struck just before a steady-clock discontinuity, superseded by the session under Yan's rule on decisions whose need a change still meets, and told to Yan in the wave's questions issue. The one lost-history path for every engine stands. #1881

## 2026-11-20 (lattice-board decoding, PR #1894)

Lines marked "Yan" are his answers in chat; lines marked "session" are the implementing session's own choices, reported to him and his to change.

- Harmonigraph decodes the lattice-board's chain-of-fifths MIDI, replacing the Note Grid patch in each project: a normal 12-TET key with the MIDI channel as a signed 12-fifth comma bank, Ch 1 holding D♭ to F♯. Yan (the window; the move itself)
- Decoding is a setting of each Tune, not of the Hub: Yan improvises in meantone beside recorded 12-TET tracks, which must stay as recorded. Yan (the use), session (the mechanism) Superseded by the #1897 entry below.
- The decoded chain sounds at its own saved Decoded fifth, apart from the lattice's Perfect fifth, so switching between meantone improvisation and composing in JI is the Note retuning selector alone and no tuning slider moves. Yan
- The Decoded fifth is a setting in the project, edited in the editor, with no host automation and no glide of held notes, in the Perfect fifth's own range. Yan
- Lattice Map keeps its twelve slots: a decoded key takes its key's slot and its comma bank is ignored, so C♯ and D♭ on one key share a slot and Map Thirds keeps choosing between them. A map that shifts a slot by a diesis per bank was built and dropped: it sounded nodes the map does not hold. Yan
- Adaptive hears a decoded Tune as a keyboard tuned in the Decoded fifth, through the keyboard check it already has, while other sources keep the Keyboard block; kept because it was cheap, and Adaptive stays outside the use. Yan (keep it in mind if cheap), session (the mechanism)
- Learn does not hear a decoded Tune: its tuning is declared, and its keys arrive in 12-TET. Session
- The Decoded fifth opens at quarter-comma meantone, 696.578¢. Session
- Adaptive fell out of use because it guessed the syntonic comma wrong and drifted, with no obvious choice in pentatonic-like chords; spelled input removes the enharmonic guess and not that one. Yan is open to another try, which is not this PR. Yan

## 2026-11-20 (Decode fifths for the project, #1897)

- Decode fifths is one setting for the project, saved with the policy beside the Decoded fifth, and every source with Retune on is decoded while it is on. This supersedes the per-Tune mechanism above: "decode fifth should be a global toggle, not per source. I don't usually want to control that setting for each track individually." Yan
- Its controls and documentation name it as the decoding for the lattice-board, a custom controller, to be left off for any other input: "this should be noted as something specific to my instruments just in case users get confused". Yan
- A 12-TET track with Retune on is decoded with the board, its Ch 1 notes read as D♭ to F♯; a note off by a Pythagorean comma is moved to the next channel's bank in the piano roll, and Retune off keeps a whole track out. No per-Tune opt-out. Yan
- The board keeps its central bank on Ch 1 rather than leaving Ch 1 to plain 12-TET, so notes drawn in the piano roll mix with the board's recorded notes on one track. Yan
- With Decode fifths on, Learn hears only sources with Retune off, the rule above that it does not hear a decoded source applied to the project's flag; Learn is not suspended while decoding is on, though with the board alone it then hears nothing while its armed ring still shows. Yan (PR #1898 review)
- A note struck decoded is heard by Learn as its raw 12-TET key once decoding stops applying to it while it is held (Decode fifths or that source's Retune turned off): no per-note record of decoding at the attack. Yan (PR #1898 review)
- No setting for which twelve fifths Ch 1 holds: Harmonigraph cannot tell the board's notes from drawn ones, so its window would have to match the board's firmware, kept in step by hand, and every recorded clip would be read again when it changed. "not sure that it's worth it if I have to keep the board in sync manually" Yan
