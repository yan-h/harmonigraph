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

Choose Lattice Map in Tuning,
pick a Saved map,
and tick Edit shape on lattice.
Each lattice click then edits the selected saved map itself:
the destination's fixed-generator MIDI class determines which assignment moves,
and the edit is saved at once,
so every passage whose Map automation selects that map plays the new shape from its next attack.
Hover previews the full correction and pitch change;
a camera drag never commits an edit.
When host automation switches Map,
clicks edit whichever map is selected at that moment — the one the lattice's dots show.
Undo shape edit steps back through the selected map's own edits,
so it never rewrites a map that is not on screen.
All maps share one history of the last 64 edits,
so heavy editing of one map can push out another's oldest steps.
An edit that changes nothing,
such as clicking a node a note already occupies,
is not recorded and does not mark the project modified.
Closing the editor keeps that history;
loading project state clears it,
because its entries name slots of the document that was replaced.
Duplicate as new map copies the selected shape into a new slot named after it,
then selects the copy through an ordinary Map parameter gesture,
so a variant is one click and the original stays as it was.
Editing is suspended outside Lattice Map mode.

Saved maps define shape only.
Use the independent Map Fifths,
Map Thirds and Map Sevenths automation lanes to move any shape in integer generator steps.
Each axis also has a Coarse lane that adds multiples of 10 without changing existing fine automation.
The coarse parameters are named Map Fifths x10,
Map Thirds x10 and Map Sevenths x10;
the editor labels the two columns Fine and Coarse.
These controls sit in the Map offsets fold.
Positive values move along the corresponding generator;
negative values move back.
Zero on all six lanes places the shape at its origin.
Recalling a map changes shape and leaves these offsets unchanged.
Editing and duplicating never record or change the offsets;
offset changes are host parameter gestures.

With the pointer over the lattice in Lattice Map mode,
the arrow keys move the map one step at a time.
Each arrow takes whichever of the fifths and thirds axes points most nearly its way on screen,
so the assignment dots follow the arrow however the camera is turned;
in the default Cabinet view Right and Left move along thirds and Up and Down along fifths.
A plain arrow steps the Fine lane and carries into Coarse past ±9,
so the total moves by exactly one;
Shift with an arrow steps Coarse alone.
Each changed lane is an ordinary host gesture,
recorded like a drag of its control.
Sevenths have no arrow and stay on their lanes.
Arrows do nothing while another control holds the keyboard.

Select a saved map to report an ordinary host parameter gesture,
or draw discrete held values for Map in Bitwig.
Use held segments:
a host ramp that explicitly sends intervening IDs selects those IDs.
There is no smoothing or interpolation in the plugin.
Map and all six offset lanes do not advertise CLAP additive modulation.

Assignments and sounding intervals shows all twelve full corrections and exact coordinates,
plus actual held-note intervals above the lowest sounding voice.
It does not judge which musical intervals a passage ought to use.
The lattice marks the next-attack mapping with a dot above each assigned node,
independently of sounding notes.
Dots use the fully sounding lettering's ink and shadow at all times,
even when a node is silent or its name is hidden.
Show map indicators in the Lattice Map controls hides those assignment dots without changing the selected map or its tuning;
the hovered edit destination and map status remain visible while editing.
Stopped edits and restored maps are visible before audio resumes,
with a pending-audio indication until adoption.
Assignment dots and edit previews are editor-only;
Render preview and offline output exclude them explicitly.

## Follow harmony (prototype)

Follow harmony lets the Hub slide the selected map by itself,
one generator step at a time,
so a chord's intervals take their simplest spellings.
It approaches adaptive tuning from the map's side:
the shape stays what you drew,
and only its place on the lattice moves.
Off is the default;
Thirds moves along thirds only;
Thirds and fifths also moves along fifths.
The parameter is `map-follow`,
named Map Follow,
with values 0–2;
it is automatable,
so a passage can switch following on and off,
and it is timestamped with the other map lanes.

With the default shape a thirds step moves one column by a diesis,
which is the error behind Ab–C sounding as a diminished fourth,
and almost never a spelling anyone means.
A fifths step moves one row by a syntonic comma,
the classic ambiguity,
so it costs more and is taken only when it clearly helps:
D F A struck together steps down a fifth,
which turns the 9/8 D into 10/9 and removes the wolf fifth against A.

The first onset of each attack group decides for the whole group.
An attack group is every onset on one input sample,
so a sequenced chord is decided as a chord,
while a chord played by hand arrives one note at a time and is decided note by note.
Releases on that sample apply first,
so a chord change on one sample is weighed against what is actually still held.
Each candidate is the current place or one step either way along each allowed axis.
Its score is how much more complex,
in bits of Tenney height,
each interval the group would make is spelled than its 12-TET class needs,
among the group's own notes and against the context,
plus the cost of the step.
The context is the one Adaptive uses:
the held notes,
or with nothing held the released ones,
each decayed on the Adaptive half-life.
A released note weighs half what a note in the chord does,
so it decides between spellings the chord leaves open,
but it cannot make the chord keep a wolf of its own.
The lowest score wins;
ties keep the current place,
then go to the candidate farther from the place the offset parameters set.

Following prefers drift.
Held notes keep the tuning they started with,
and a note just heard keeps its pitch where the chord allows,
so a common tone holds still and the map moves around it rather than snapping back.
D held from D minor into G major takes G and B a comma down with it rather than sounding a wolf against it,
and the same happens with that D just released.
I–vi–ii–V–I therefore ends a syntonic comma below where it started,
and a chain of major-third relations drifts a diesis per lap,
as in any adaptive just tuning.
The map returns only when the context asks for it:
G major straight after C major keeps the G just heard rather than stepping on.

The follow offset adds to the offset parameters and never writes them,
so it neither fights nor records into their automation.
It returns to zero when following is turned off,
when the retuning mode or the follow mode changes,
and wherever Adaptive clears its released memory:
after the silence timeout,
at Stop and loop resets when those are enabled,
and at an explicit session reset.
A mode change takes effect at the next attack,
which starts from the automated place with an empty context,
so switching following off and on again with nothing played starts over.
Selecting another saved map keeps the offset,
so a passage that switches shapes keeps its place on the lattice.
The lattice and the assignments list show the map where it sounds,
including the follow offset,
and the Tuning pane reports how far following has moved it.
Shape edits are made against that same placement.

This is a prototype.
The score judges 5-limit consonance only:
a septimal interval in a shape counts as its excess over the simplest 5-limit spelling of its class,
or as zero when it is simpler,
so following neither seeks out nor avoids sevens.
A node in the context counts once,
at its strongest weight,
so an octave doubling adds nothing.
Changing either mode clears the context,
so notes held or heard before the change are not weighed.
In Lattice Map mode the Hub's diagnostic count of context voices now counts mapped voices too.

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
Editing a shape rewrites its slot in place and keeps its identity;
Duplicate as new map is how a revision gets an identity of its own.

The musical `lattice-maps` document is a plugin-persisted parameter field,
independent of editor appearance and layout.
It is restored and saved by the ordinary host state path even with the editor closed.
Each nested serialized struct has container-level defaults.
Malformed geometry is refused as an unavailable map,
never silently repaired to a different chosen copy.
Names are exposed through the Map parameter's value text;
document edits request a host state-dirty notification and value/text rescan.

Each map holds twelve coordinates relative to C.
Each axis’s fine and coarse parameters add to its absolute region position.
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
each ranges from −9 to +9 in single steps and defaults to zero.
Each axis also has an independent Coarse lane from −90 to +90 in steps of 10,
with IDs `map-fifths-extension`,
`map-thirds-extension` and `map-sevenths-extension`.
The offset and extension add together,
so every integer total from −99 to +99 is reachable.
For example,
Fine +4 and Coarse +20 give a total of +24.
Use Coarse when a passage outgrows the fine lane;
existing automation stays intact because neither lane's range changes.
The editor shows both controls and the total separately for each axis.

All six plain saved parameters range from −9 to +9 and default to zero;
coarse counts are multiplied by 10 only when displayed or added to the total.
CLAP exposes stepped indices 0–18 with neutral index 9.
The former ±4096 offset lanes are narrowed in this release:
old host automation is reinterpreted,
and saved offsets outside ±9 are clamped.
Missing lanes restore to zero.
There is no automatic migration of old envelopes.
Dynamic range expansion was rejected after [the Bitwig probe](evidence/1051/README.md) doubled existing automation.
The earlier prototype's per-map `position` field is removed:
previously captured shapes remain,
but their saved positions are ignored and must be set using the offset parameters.

Temperament,
axis values and reference pitch remain shared musical settings.
Selecting or capturing a map cannot restore another tuning.
Tuning entries recognize shared temperament links;
map selection does not change them.
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

The UI owns the map document and the editor's edit mode and undo history behind separate locks;
the audio thread reads only the document.
At callback adoption audio takes bounded nonblocking reads and copies only fixed geometry into its own bank.
A contended edit waits for a later callback;
audio never waits or copies names.
Each timestamped history entry captures complete geometry and effective shared tuning,
so a later document edit cannot reinterpret an older attack.

The CLAP wrapper already retains timestamped input and walks configuration before performance.
At adoption the owner seeds Map,
mode and all six offset components from host parameters,
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
