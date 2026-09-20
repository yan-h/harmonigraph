# Lattice Map tuning

Implements the passage workflow in [#889](https://github.com/yan-h/harmonigraph/issues/889).
Use the existing Harmonigraph Tune instances before instruments and one Harmonigraph Hub.
Retune and Show remain per-source controls.
Note retuning is shared by the Hub:
Pass through preserves player pitch,
Adaptive uses the existing policy,
and Lattice Map assigns each incoming note from explicit geometry,
choosing its slot by the 12-TET key the note actually sounds.
Adaptive remains the default.
Mode is saved but is not automatable in this prototype.

## Compose passages

Choose Lattice Map in Tuning.
Audition working copy makes a separate copy of the currently selected saved map.
Its shape edits remain active through loops and Map selection automation until Return to arrangement.
The three offset automation lanes remain live during audition.
Closing the editor does not end audition;
loading project state does.
Audition and its bounded 64-edit undo history are transient and are not saved.

Saved maps define shape only.
Use the independent Map Fifth Offset (steps),
Map Third Offset (steps) and Map Seventh Offset (steps) automation lanes to move any shape in integer generator steps.
The same controls are always available in Lattice Map mode without entering audition or capturing a map.
Positive values move along the corresponding generator;
negative values move back.
Zero on all three places the shape at its origin.
Recalling a map changes shape and leaves these offsets unchanged.
Edit shape enables one destination click on the lattice:
the destination's fixed-generator MIDI class determines which assignment moves.
Hover previews the full correction and pitch change;
a camera drag never commits an edit.
Undo restores the previous shape;
offset changes are host parameter gestures.
Editing is suspended outside Lattice Map mode.

Capture new map appends the working shape under a name.
It never captures the three live offsets.
It does not select that map or emit Map automation.
Return to arrangement resumes the host-selected map.
Select a saved map to report an ordinary host parameter gesture,
or draw discrete held values for Map in Bitwig.
Use held segments:
a host ramp that explicitly sends intervening IDs selects those IDs.
There is no smoothing or interpolation in the plugin.
Map and the three offsets do not advertise additive modulation.

Assignments and sounding intervals shows all twelve full corrections and exact coordinates,
plus actual held-note intervals above the lowest sounding voice.
It does not judge which musical intervals a passage ought to use.
The lattice outlines the next-attack mapping independently of sounding notes.
Stopped edits and restored maps are visible before audio resumes,
with a pending-audio indication until adoption.
Map and adaptive outlines share one painter and style.
Outlines,
MIDI annotations and edit previews are editor-only;
Render preview and offline output exclude them explicitly.

## Saved identity and geometry

The host parameter ID is `lattice-map`,
with 128 slots numbered 1–128 in the UI and values 0–127 in CLAP.
Slot indices are stable identities.
Rename and display-order changes do not change them.
Delete leaves a tombstone;
no slot is ever recycled.
An unused,
deleted or invalid slot adds no correction and the UI reports it as unavailable.
Capturing fails visibly at capacity.
Saved maps cannot be overwritten in place in this prototype;
revisions are captured as new identities.

The musical `lattice-maps` document is a plugin-persisted parameter field,
independent of editor appearance and layout.
It is restored and saved by the ordinary host state path even with the editor closed.
Each nested serialized struct has container-level defaults.
Malformed geometry is refused as an unavailable map,
never silently repaired to a different chosen copy.
Names are exposed through the Map parameter's value text;
document edits request a host state-dirty notification and value/text rescan.

Each map holds twelve coordinates relative to C.
The three offset parameters supply absolute region position independently.
The starting rectangle has fifth coordinates −1 through 2,
third coordinates 0 through 2,
and seventh coordinate 0:

| Third column | Four fifth rows |
|---|---|
| 0 | F, C, G, D |
| 1 | A, E, B, F♯ |
| 2 | C♯, G♯, D♯, A♯ |

Each relative coordinate and position axis is bounded to ±4096.
Only MIDI labels wrap:
`(7f + 4t + 10s) mod 12`.
The correction in microcents is the shared C offset plus each absolute node coordinate times the corresponding shared axis deviation from 700,
400 or 1000 cents.
No nearest-pitch or nearest-octave operation occurs.
Every MIDI octave repeats that full correction,
even after drift exceeds an octave.
Exact geometric copies survive temperament changes and project recall.
Offset IDs are `map-fifths`,
`map-thirds` and `map-sevenths`;
plain saved values range from −4096 to 4096 and default to zero.
CLAP exposes stepped indices 0–8192 with neutral index 4096;
parameter text displays the signed musical steps.
The earlier prototype's per-map `position` field is removed:
previously captured shapes remain,
but their saved positions are ignored and must be set using the offset parameters.

Temperament,
axis values and reference pitch remain shared musical settings.
Selecting or capturing a map cannot restore another tuning.
Auto retains the existing shared axis/lock semantics.
Learn is suspended while Lattice Map is active,
so preceding notes cannot move its reference or axes.
Leaving or entering Adaptive clears its context at the next attack group;
it never clears the sounding voices' frozen corrections.

## Which slot an incoming note takes

An onset's whole sounding pitch chooses its slot:
key number,
per-note tuning and channel bend added together and then rounded to the nearest 12-TET semitone.
A keyboard that already applies its own temperament — quarter-comma meantone,
an MTS scale — therefore lands on the key it is playing rather than on whatever its detune drifts past.
A source further than half a semitone from its written key selects the key it actually sounds;
the map holds twelve slots and has nothing else to offer a pitch that far out.

The map then states an absolute pitch rather than an increment.
The emitted correction is the difference from what arrived to the map's own pitch,
so the incoming tuning is spent selecting a slot and is not added to the map's offset a second time.
An E arriving already 13.686¢ flat onto a map whose E is 5/4 receives no further correction at all.
Adding to the arriving pitch instead would move every note off the map by exactly the source's own tuning,
which is [#891](https://github.com/yan-h/harmonigraph/issues/891).

Reading a standing channel bend as part of the arriving pitch is the same rule Adaptive applies,
so the two engines never disagree about what was played.
It has a cost worth stating:
a bend held at the attack is absorbed into the frozen correction,
and returning the wheel to centre afterwards moves that voice by the amount the bend was worth.
A per-note tuning event and a performed bend are one number on the wire;
telling them apart would need a declared source-tuning baseline the input does not carry.
Expression after the onset stays live as a change from it,
under the existing frozen-correction contract.

## State ownership and ordering

The dependency order is explicit geometry,
persisted map document and backend API,
audio adoption and timestamp lookup,
Hub assignment,
then editor controls and annotations.
There are no parallel mutating streams.

The UI owns the map document and audition working copy behind separate locks.
At callback adoption audio takes bounded nonblocking reads and copies only fixed geometry into its own bank.
A contended edit waits for a later callback;
audio never waits or copies names.
Each timestamped history entry captures complete geometry and effective shared tuning,
so a later document edit cannot reinterpret an older attack.

The CLAP wrapper already retains timestamped input and walks configuration before performance.
At adoption the owner seeds Map,
mode and all three offsets from host parameters,
then observes every Map/offset event and resolved tuning edit at its absolute input sample.
The last configuration at a coincident sample wins before all attacks at that sample,
even if the host lists a note first.
The Hub looks up the attack's original input sample,
not its delayed output time.
History retains 8192 distinct effective states without audio allocation.
It survives song-time seeks and resets on steady-clock discontinuities.
Late records can use previous callbacks' states;
a record outside retained or known history raises the policy fault only under Lattice Map,
where the map that sample ran under is exactly what was lost and guessing at one would retune the onset to a shape nobody selected.
Adaptive decides from the block configuration, which is still in hand, so it assigns from that and raises nothing.
This does not extend the transport's existing correction deadline or callback-order latency requirements.

Map attacks use this timestamped shared tuning;
Adaptive retains its existing block-adoption semantics.
Recording segments also split at Map-mode shared-tuning changes,
so replay sees the same axis-change boundaries.
Each accepted voice carries its resolved onset configuration,
chosen node and frozen correction through display and take publication.
Existing player expression remains live.
A chased or retriggered note is a new attack using current state;
there is no original-onset recovery.

## Validation

Core fixtures cover fixed label rotation,
the fifty-fifths example,
unwrapped drift beyond an octave,
MIDI octave repetition,
JI versus meantone substitution at exact geometric copies,
and slot selection by rounded key with no second correction of a pre-tuned source.
Production CLAP fixtures cover coincident automation across buffer sizes and both Tune/Hub callback orders,
signed fifth/third/seventh offsets,
held-note retention,
new/chased attacks,
Off with player expression,
a keyboard arriving in its own tuning,
a detune large enough to select the neighbouring key,
a channel bend standing at the attack,
and editor-closed host project recall.
Document tests cover naming,
reordering,
tombstones and exact-copy serialization.
These are synthetic host tests;
Bitwig playback and Note Chase should also be auditioned with the loadable build before treating the prototype as host-qualified.
