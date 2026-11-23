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
- The decoded chain sounds at its own saved Decoded fifth, apart from the lattice's Perfect fifth, so switching between meantone improvisation and composing in JI is the Note retuning selector alone and no tuning slider moves. Yan The selector alone stands; that no slider moves is superseded by the entry on switching below.
- The Decoded fifth is a setting in the project, edited in the editor, with no host automation and no glide of held notes, in the Perfect fifth's own range. Yan
- Lattice Map keeps its twelve slots: a decoded key takes its key's slot and its comma bank is ignored, so C♯ and D♭ on one key share a slot and Map Thirds keeps choosing between them. A map that shifts a slot by a diesis per bank was built and dropped: it sounded nodes the map does not hold. Yan
- Adaptive hears a decoded Tune as a keyboard tuned in the Decoded fifth, through the keyboard check it already has, while other sources keep the Keyboard block; kept because it was cheap, and Adaptive stays outside the use. Yan (keep it in mind if cheap), session (the mechanism) Superseded by the 2026-11-22 strip entry below.
- Learn does not hear a decoded Tune: its tuning is declared, and its keys arrive in 12-TET. Session Superseded by the 2026-11-20 entry suspending Learn while Decode fifths is on.
- The Decoded fifth opens at quarter-comma meantone, 696.578¢. Session
- Adaptive fell out of use because it guessed the syntonic comma wrong and drifted, with no obvious choice in pentatonic-like chords; spelled input removes the enharmonic guess and not that one. Yan is open to another try, which is not this PR. Yan

## 2026-11-20 (Decode fifths for the project, #1897)

- Decode fifths is one setting for the project, saved with the policy beside the Decoded fifth, and every source with Retune on is decoded while it is on. This supersedes the per-Tune mechanism above: "decode fifth should be a global toggle, not per source. I don't usually want to control that setting for each track individually." Yan
- Its controls and documentation name it as the decoding for the lattice-board, a custom controller, to be left off for any other input: "this should be noted as something specific to my instruments just in case users get confused". Yan
- A 12-TET track with Retune on is decoded with the board, its Ch 1 notes read as D♭ to F♯; a note off by a Pythagorean comma is moved to the next channel's bank in the piano roll, and Retune off keeps a whole track out. No per-Tune opt-out. Yan
- The board keeps its central bank on Ch 1 rather than leaving Ch 1 to plain 12-TET, so notes drawn in the piano roll mix with the board's recorded notes on one track. Yan
- With Decode fifths on, Learn hears only sources with Retune off, the rule above that it does not hear a decoded source applied to the project's flag; Learn is not suspended while decoding is on, though with the board alone it then hears nothing while its armed ring still shows. Yan (PR #1898 review) Superseded by the entry on switching below.
- A note struck decoded is heard by Learn as its raw 12-TET key once decoding stops applying to it while it is held (Decode fifths or that source's Retune turned off): no per-note record of decoding at the attack. Yan (PR #1898 review) Since the 2026-11-20 entry suspends Learn while Decode fifths is on, only turning Decode fifths off reaches this.
- No setting for which twelve fifths Ch 1 holds: Harmonigraph cannot tell the board's notes from drawn ones, so its window would have to match the board's firmware, kept in step by hand, and every recorded clip would be read again when it changed. "not sure that it's worth it if I have to keep the board in sync manually" Yan

## 2026-11-20 (lighting the lattice-board)

- Harmonigraph sends the board its lit keys itself, over CoreMIDI, rather than through a Bitwig hardware-out device or a controller script: Bitwig turns a Tune's per-note tuning into MPE channels, which scrambles the comma banks, and its controller API gives a playing note only a pitch and a velocity. Yan
- A note lights the key nearest its sounding pitch in the board's own tuning (the Decoded fifth, the lattice's C offset), not the key played or drawn: a note a Lattice Map moved lights the key it sounds on, and a 12-TET track its nearest key. Yan Superseded under Adaptive by the 2026-11-22 strip entry below, pending Yan's listening check.
- Every note the lattice draws lights the board, from any source and from the Tape's cues. Yan
- No setting of its own: the board lights while Decode fifths is on. Yan
- Keys pressed and keys sounded share the board's one highlight for now; a separate tint, a firmware change, waits until it has been used. Yan
- A note lights the nearest key the board has, searching the 25 fifths its 5x25 layout holds (E𝄫 to C𝄪), which `board.rs` mirrors from the firmware: Yan agreed to "the board key that would sound nearest". A first build searched three channel banks to keep nothing in step with the firmware, and review (#1899) found notes left dark with a board key 3¢ away, and at third-comma whole regions of central keys. Yan's words (the board key), session (the span)

## 2026-11-20 (switching between the board and a map, #1895)

Yan: "I want to be able to easily go between improvising in meantone while visualizing what I'm playing on the lattice, and improvising over a justly tuned piece in the piano roll while also visualizing what I'm playing in the lattice", and "learn doesn't work for making the lattice match the tuning of the lattice board". He took the proposal posted on #1895 ("that all sounds good").

- Under Pass through with Decode fifths on, the lattice is drawn in the lattice-board's tuning: the Decoded fifth with Meantone linked, the seventh as saved unless Marvel derives it. The saved axes are left alone, so the Note retuning selector alone switches both the sound and the picture between meantone improvisation and a just Lattice Map. Yan
- The bars show the tuning drawn, so they move when the selector does; there Tuning's Perfect fifth edits the Decoded fifth. Yan. The third, the seventh, both links and the presets are greyed there, since they would edit saved intervals nobody sees. Session
- A take records the tuning drawn: the take recorder draws each configuration it writes with the engine in force at its sample (`Owner`, `crates/harmonigraph-plugin/src/configuration.rs`), while the Hub assigns from and a save stores the saved axes. Session
- Learn is suspended while Decode fifths is on, under every engine, rather than taught to hear the board's decoded pitch: that would need each voice to keep its decoding, learn only what the Decoded fifth already declares, and still hear the 12-TET tracks beside the board. Yan
- Not taken: a lattice tuning kept per engine (a second copy of the Decoded fifth kept equal by hand), a preset button for the board's tuning (two actions per switch), and a tuning per saved map (Map automation would retune the axes, and Pass through is left unsolved). Session, put to Yan with the proposal
- Decode fifths and the Decoded fifth sit in a section of their own, Lattice-board, after the engines' controls, which a fresh editor opens folded, "so users don't get confused". Yan. Folded through the fresh editor's default fold set rather than a mechanism that folds a section by default, so an existing project keeps its saved folds and shows the section open until folded once. Session; kept by Yan over saving folds as changes from each section's default (#1900 review) The placement is superseded by the 2026-11-22 UI pass below.
- Decode fifths can stay on while switching to Lattice Map: a map ignores the decoding, so switching is the Note retuning selector alone. Its hint says so. Yan
- Under Lattice Map with no map in the selected slot, the board sounds its chain at the Decoded fifth while the lattice and a take draw the saved axes; the picture does not follow the sound there. Yan (#1900 review, leaving it unless he plays from an empty slot)

## 2026-11-22 (Analyzer Audio Input saved as an index, #1886)

- A saved host parameter value that does not decode, such as Analyzer Audio Input's string id from before it saved an index, takes its row's default with a notice (a Console line in a Hub, a log line in a Tune) rather than refusing the whole load; this extends the 2026-11-18 rule that a missing, wrong-typed or unknown value takes the default. Each load of an older save shows the notice once, Main included, until the project is saved again. Yan, 2026-11-22, #1886

## 2026-11-22 (one presentation clock, #1885)

- Steady samples the plugin never processed take no presentation time: a forward gap in the host's counter closes, and the note after it sits on its audio in the editor and in a take. Yan, 2026-11-22, answering #1885's question yes
- One owner, `PresentationClock` (`crates/harmonigraph-plugin/src/presentation.rs`), produces each callback's start and end and the steady-sample-to-presentation mapping for live analysis, Tape, notes, reset markers and take boundaries; the take format stays. Yan, 2026-11-22
- The gap closes in the plugin's presentation clock, not in the shell's steady clock: a Tune's records are matched to the Hub's callbacks on the host's counter, which each instance closing its own gaps would set apart. Session (PR for #1885)
- A row's note times never go back: a delta the Hub schedules before the row's latest published time is recorded at that time. A record read late across a closed gap or a lost-input callback stays in order, and a take recorded while a Tune's delay is lowered exports, its next notes up to the old delay late, rather than refusing. Yan, 2026-11-22, #1885 Q1
- A valid callback whose host input was lost takes no picture time either, so notes after it stay on their audio. Yan, 2026-11-22, #1885 Q2

## 2026-11-22 (a tuning field that does not parse, #1927 A)

- A saved `musical-settings` or `tuning-instance` that does not parse opens at its default on its own, with a line in the log and, in a Hub, a Console line naming it, and the rest of the project loads. This supersedes the `persistence-contract` skill's rule that such a field refuses the whole restore with a line in the log: the untouched editor then saved the fresh look over the project's at the next save. A full tuning edit queue still refuses the restore whole, as the issue's option said it would stay. Yan, 2026-11-22, #1927

## 2026-11-22 (Adaptive's strip for the lattice-board, #1896)

Yan, in chat: "Let's try the strip. It seems promising enough to demo." The design pass on #1896 put seven questions with recommendations; the build takes the recommendations for now, and none is decided until Yan has played it at the board. Each line below is the session's. Yan heard the fixed strip and the follower on 2026-11-22 ("strip seems useful"; then the two requests below); the follower changed after, and its lines are pending his next demo.

- With Decode fifths on, Adaptive tunes a decoded onset to its spelling's one just copy in a strip four fifths wide, `node(k, s) = (k − 4j, j)` with `j = floor((k − s) / 4)` (`policy::strip`); the search, its memory and the keyboard check are not consulted. Session; heard 2026-11-22, and the follower changed after
- The strip's flat column is −1 plus Lattice Map's Position fifths (fine and coarse lanes together), read under Adaptive as under Lattice Map and clamped to flat columns −11..8, so every column lies within eleven fifths of C (no note more than about 59¢ from its key); the thirds and sevenths lanes are not read. A lane move applies from the next onset and moves no held note. Session; heard 2026-11-22, and the follower changed after
- The wolf, a fifth a comma narrow at the strip's sharp edge (D–A at Position 0), is accepted at its fixed place, sus and pentatonic chords included; neither a split column nor a column moved by chord is built. Session; heard 2026-11-22. The column moved by chord is superseded by the follower's excursion (the 2026-11-23 line below)
- The search stays for Decode fifths off, with all its settings; with Decode fifths on, Keyboard, Pitch flexibility, Search radius, Search axes and Weight per octave are greyed under the strip's block, which shows its columns and the Position fifths lanes. Session; heard 2026-11-22. The greying is superseded by the 2026-11-22 UI pass below.
- Under Adaptive, a held note lights the board key it was played on, when the board has that key, and otherwise the key nearest its pitch; under the other engines the 2026-11-20 rule stands. This supersedes, for Adaptive, the 2026-11-20 line "A note lights the key nearest its sounding pitch … not the key played or drawn": a strip note in the fourth column or beyond is 21.5¢ from its own key and 19.6¢ from its enharmonic neighbour's. Session; heard 2026-11-22, and the follower changed after
- Stakes: Adaptive with Decode fifths on is within the use; Adaptive's search, with Decode fifths off, stays outside it. Session's proposal from the design, recorded in AGENTS.md pending Yan's listening check
- Follow spellings, a switch on top of the set key, on by default and saved with the policy (absent from an older save reads on): the strip slides a fifth sharp when the spelling just past its key's seven (F♯ over F C G D A E B) has a count of at least 3 and at least twice that of the spelling at the other edge (F), mirrored flat, and slides back toward the Position once the spelling that step pushes out counts under a quarter. Counts halve every 8 s of attack-to-attack time, apart from Adaptive's Half-life; one slide per attack. The count rule reads spellings and never an interval; the chord rule added later reads the fifths among the sounding spellings. The constants sit at the top of `policy::strip` for the ear. Session; heard 2026-11-22, and the follower changed after
- The slide follows the fifths, retuning one column by a comma, rather than wrapping to the nearest home: E major reached from C sits on the Pythagorean E, and home again is identical. Session; heard 2026-11-22, and the follower changed after
- A slide that would move a held strip note's column waits for an attack after that note ends; a lane move and a context reset do not wait. Every context reset (Silence reset, Reset on Stop, Reset on loop, a change of engine) returns the strip to the Position. Session; heard 2026-11-22, and the follower changed after
- 12-TET tracks with Retune on are decoded with the board and count toward the key; Retune off keeps a track out. Session; heard 2026-11-22, and the follower changed after
- After playing the follower: "following doesn't seem eager enough. I run into wolf fifths when modulating up or down by fifths playing triads only". Yan, 2026-11-22. So the follower decides each attack group once, before its notes are tuned, between staying and a fifth either way, never moving a held note's column. Where a fifth among the sounding spellings (struck and held) is a wolf, it slides where that leaves none: the count rule's way first, then back toward the Position, then toward the wolf, and never away from it, which with chords rolled by hand walked the strip from the key a fifth a chord (design review on #1941); a roll whose held notes pin the slide keeps its wolf. Otherwise the count rule steps, adding no wolf, and a step back leaves alone a column a sounding note sits in. Yan asked for eagerer following; the coordinating session's request said "one slide toward it", and the session preferred the count rule's way and the way home, so a vi after a ii that slid comes home rather than moving the tonic a comma (design review on #1941). Pending his next demo
- The follower splits the key from each chord's excursion (2026-11-23, the coordinating session deciding for Yan while he was away, from the design review of #1941 at `244e2c5`). The key moves a fifth at a time by the count rule alone. Each attack group takes an excursion of −1..=1 fifths around it: one that holds every held note at the node it sounds at, if one does; then the fewest wolves among its fifths; then the fewest common tones with the last group retuned; then the fewest sounding spellings retuned from where the strip last stood; then none; then nearer the Position. The key moves only where its excursions can still hold every held note and add no wolf the group did not have. So the strip stands at most a fifth from the key, a run of chords cannot walk it away (A–D–G then D–E–G–A no longer reaches two fifths out), a lane move does not retune a held note's fifths, and a key move adds no wolf to the group it is decided at. This supersedes the candidate order in the line above. Session, pending Yan's next demo. The excursion's freedom for every group is superseded by Yan's (b) below, and the excursion itself by the 2026-11-23 simplification.
- The chord rule acts only for an attack group holding a spelling outside the key's seven-spelling window, read as the count rule reads spellings (F♯ against C's window, or B♭); a group whose spellings all lie inside the window takes the key's own strip. Triad modulation still slides at its first chord with a foreign spelling (D–F♯–A or B♭–D–F from C, A–C♯–E from G), toward that spelling's side; pentatonic and sus voicings never slide, at home or at a followed key; a diatonic ii keeps its wolf. Yan, 2026-11-23, choosing (b) on #1941. This supersedes, for groups inside the window, the chord rule in the two lines above. The tie-break toward the foreign spelling's side, so B♭ major from C slides flat, is the session's, as is its place before the fewest spellings retuned and no excursion, so a lone C♯ after D major stays in D major's strip. What (b) guarantees is that a key move adds no wolf to the group it is decided at; a later group inside the moved key's window takes that key's wolf, which may be a fifth the old key left pure (A–E in G after F♯ three times over a held F). A struck spelling is weighed at its copy in the strip as well as where any held copy of it sounds, and the key stores only movement the Position clamp lets through. The session, 2026-11-23, from the final review of #1941 at `7e0aaf0`. The excursion, its ranking and the copy rule are superseded by the 2026-11-23 simplification.
- "could we have some sort of indicator for the strips in use? maybe reuse the ones used by lattice map?" Yan, 2026-11-22. Under Adaptive with Decode fifths on, Lattice Map's map marks stand on every visible node of the strip's four columns, and move when it slides. Session; heard 2026-11-22, and the follower changed after

## 2026-11-23 (the follower simplified, #1941)

Yan, in chat, after seeing the numbers (`policy::strip` at 633 lines against a design estimate of about 100, six review rounds each finding new interactions between the key, the excursion and its ranking), asked for the follower to be rebuilt simple. It supersedes the per-chord excursion, the wolf ranking and the copy rule in the 2026-11-22 lines above; the count rule, the saved switch, the marks and the UI pass stay.

- One state, the key, a whole number of fifths from the Position. Each attack group, before it is tuned, moves the key the fewest fifths that put every struck spelling inside the key's seven; a group no seven spellings hold moves nothing. A lone note moves it as a chord does, held notes keep the node they sound at and veto nothing, and the key keeps only movement the Position clamp lets the strip sound. Yan, 2026-11-23: "when I move a triad up a fifth, arpeggiated so the order of notes is random and staggered, it should always shift the map upwards if one of the triad notes is above the current map … this assumes I'm only playing triads", and asking for further simplification. This supersedes, the same day, the coordinating session's chord rule (two spellings on one sample), the count rule with its four constants, the held-column veto and the reset of the edge count; a passing tone outside the key now moves it, and the key never returns to the Position by time.
- One state, the strip's place. Each attack keeps it if the struck notes' fifths and minor thirds sound pure against each other and the context, and otherwise moves it the fewest fifths that make them pure. The context is the held notes at the nodes they sound, or with nothing held the last note's node. Yan, 2026-11-23, after the comparison below: "drift is ok"; then "go with recommendations": drop the step that moved the key for a spelling just past its seven, and read one note back when nothing is held. This supersedes, the same day, the key with its just-past step and the one-fifth excursion around it, and Yan's earlier "it should always shift the map upwards": D major after G major, arpeggiated A then D then F♯, now moves the strip flat, keeping D pure under the held A, the same moment as "if I have FAC held and D plays, it should move down". A diatonic ii is pure, its D 10/9 against 9/8 in V, superseding for triads "a diatonic ii keeps its wolf" in Yan's (b).
- The comparison, by simulating major and minor triads moved by fifths in every arpeggio order, held and staccato, and 18,000 random triads from C major, A♭ major and F minor, and C major with secondary dominants. The just-past step left about 1 in 60 held chords impure on their second pass, as it moved the strip without checking the held notes; without it, every triad was pure from its second pass and all but 3 on their first. Checking fifths alone missed the Pythagorean minor third; placing each note at its copy nearest the context by lattice distance, as Adaptive's search does, left impure triads; a context of two or three notes back in staccato caught Yan's staccato C E♭ G then F but left about 1 in 13 staccato chords impure on their first pass. Yan's staccato C E♭ G then F and F A C then D therefore move only when held. Session
- A held note never changes its tuning when the strip moves, so the first time an arpeggio crosses into a new strip, a note struck before the move keeps the old strip's pitch until it is released. Yan, 2026-11-23: "held notes should not change tuning when strip moves ever"
- The follower resets where Adaptive's memory does (`forget_history`: Silence reset, Reset on Stop, Reset on loop, a change of engine, Reset all voices), at main's moment: a loop's reset waits for the next retuned decoded attack, after its sample's releases, and before that group is read. Session

## 2026-11-22 (the Tuning pane's UI pass, #1941)

Yan, in chat, on a Fable review's UI pass for #1941: "ui changes should go in, I don't need to review it, go with what fable thinks".

- A row is greyed while it shows a value in force and hidden while it decides nothing. Under Adaptive with Decode fifths on, Pitch flexibility, Search radius, Search axes, the Keyboard block, Weight per octave, Half-life and Same-note tolerance are hidden, with one line saying the search's settings return with Decode fifths off; Silence reset and the two reset checkboxes, which return the strip to the Position, sit in the Strip block. Tuning's intervals stay greyed under Pass through with Decode fifths on, since they show the third and seventh in force. This supersedes the greying in the strip entry above. Yan (the pass), Fable review (the rows)
- The Lattice-board section stands between Note matching and Note retuning, so its switch stays put and everything it changes is drawn below it; a fresh editor still opens it folded. This supersedes "after the engines' controls" in the 2026-11-20 line. Yan (the pass), Fable review (the place)
- The Strip block reads its columns and its wolf ("F C G D · wolf D–A") and a Moved line drawn at zero too, as Follow harmony's is. Yan (the pass), Fable review (the readout)

## 2026-11-22 (the main thread owns the tuning configuration, #1938)

Yan took the rework, chose Learn shape C, "I want to keep it around in case I use MPE based boards", and chose that tuning edits show and save at once while the audio engine is off.

- The main thread owns the tuning record, as it owns the Lattice Map document: the editor's edits, Learn and a restore write it through one door (`Shared`, `crates/harmonigraph-plugin/src/shared.rs`), and each write publishes its configuration through a triple buffer audio reads at each callback's start, wait-free and without the document's lock, whose release can take a lock of its own while a writer is parked. Yan; the triple buffer was the Codex review's, approved by the coordinating session on #1942
- An edit shows, saves and marks the project when it is made, with or without audio running; "Tuning change pending audio adoption" means shown and saved, not yet sounding. Yan
- Learn runs in the input worker, from the notes the lattice holds, every held one but the tape's, a hidden source's included, so it learns with the window covered or closed. It runs under Pass through alone, with Decode fifths off, and its result applies only if the document still lets it act. Yan chose C; the worker rather than a frame, and the tape and hidden-source rules, are the red team's revision posted on #1938. Session
- Superseded, since the audio owner they rest on is gone: #1934's line above that a full tuning edit queue still refuses the restore whole, and the `persistence-contract` skill's matching sentence; #1314 Q1's "Only active retuning reserves the axes as its target", since Learn no longer runs under Adaptive; #1751's "Learn reads the Hub's rows" and the Learn clauses of #1749 and #1795; #1798 F2's sentence that a pane edit within a block of a Learn result may revert it. Session, on Yan's answers
- Learn hears each note at its incoming pitch, a tuner's frozen correction taken back off, so a chord held across a switch from Lattice Map to Pass through teaches its incoming pitch, the rule #1898 set for Decode fifths. Yan, 2026-11-22, on #1942
- When processing stops, the notes held then leave the lattice and the board's key lights go out at once, for every reader, Learn included, not only for Learn: `Plugin::stop` publishes a session reset in the note stream, which ends every live source's presses and leaves the tape's, and a restart's notes, published after it, stay held. This supersedes the line this entry first carried, that Learn alone stopped hearing them through a cut of its own. Yan, 2026-11-23, #1948
- Not protected, each cheap: a faster-than-real-time bounce with Learn armed records each learned change up to the bounce speed times 20 ms late, and two runs can differ; a chord that starts and ends between two worker polls can go unlearned. Yan kept both, 2026-11-22, on #1942
