# Adaptive tuning laboratory

A standalone desktop simulator for the [moving-neighbourhood design](../../docs/adaptive-tuning-design.md).
It runs independently of the Rust plugin and does not install or load a DAW build.
The model is a testable first hypothesis, not the final adaptive policy.

## Run

From the repository root:

```sh
python3 -m http.server 8765 --bind 127.0.0.1 --directory tools/adaptive-tuning-simulator
```

Open [the laboratory](http://127.0.0.1:8765/).
Serve the folder over HTTP rather than opening `index.html` directly: modules and the background reachability worker need a server origin.
There are no package installs, CDN resources, telemetry or external service dependencies.
A current browser with ES modules, Web Workers and optional Web Audio is sufficient.

Choose an example and use **Step**, **Play** or **Run to end**.
The position slider replays the experiment up to a particular event.
Parameter changes also replay from the original starting context; they do not retroactively change a note inside a single performance.
The event cadence is for stepping through decisions, not a sample-accurate musical transport.
`wait` advances virtual time immediately; the playback interval controls how quickly you inspect events.

**Try the next note** accepts arbitrary pitch, including `E5`, `Eb3`, `E5+7.82c`, `7607.82c` (absolute cents) and `76.0782m` (fractional MIDI note).
The convention is C4 = MIDI 60, A4 = 6900 cents = 440 Hz.
The displayed lattice node is its nominal spelling; the adjacent output pitch is the actual sounding frequency described relative to 12-TET.
Extended accidentals use compact counts, for example `D♯10`; the lattice coordinates remain definitive.

The preview leaves context untouched.
**Play note** adds a real event, **Release** acts on one voice, and **Release all** releases every voice.
Manual actions append at the current cursor and replace any future events, so rewinding and trying another note creates a new continuation.
**Sound** enables a quiet sine-wave audition of the current sounding notes; it is off initially.
The audition omits frequencies below 10 Hz or above 20 kHz, but the mathematical simulation retains their pitches.

The score inspector can show the next-input preview or the frozen decision for the last actual onset, including the context and moving reference used at that attack.
The displacement plot records each onset's full output-minus-input correction.
It oscillates slightly inside each chord; root-to-root displacement is the relevant quantity for the diesis examples.

## The model

Coordinates are `[fifths, major thirds, harmonic sevenths]`, with just axis sizes `1200 log2(3/2)`, `1200 log2(5/4)` and `1200 log2(7/4)`.
C3 at 4800 cents is coordinate `[0,0,0]`.
Absolute output register is stored separately from lattice coordinates.

### Context and eligibility

Every contribution decays on one clock.
Let `newest` be the latest attack in context and `t` a contribution's own attack, held or released.
Its base weight is `0.5^((newest - t) / halfLife)` for a held note and `released × 0.5^((newest - t) / halfLife) × newNote^n` for a released one, with no time decay when the half-life is zero.
`n` counts the notes assigned a lattice node that no held or remembered note occupied since that entry was released; repeating a node, in any octave, does not count.
Every weight shares that clock and the score normalizes them, so waiting changes no decision and a chord's simultaneous onsets weigh alike.
A release does not restart a note's age, so letting go of a note never raises its weight.
Released memory keeps the 24 released entries struck most recently, counted separately from held voices, which are never evicted to make room.
The 24 is a fixed storage bound with no control: at the default one-second half-life, entries beyond it carry under 2% of the weight even at four notes a second.

The same-note tolerance compares actual absolute onset pitches, including register.
A repetition refreshes a contribution rather than multiplying its weight; different octaves and diesis-shifted returns remain distinct under the default 0.5-cent tolerance.
An attack supersedes a matching released entry; a release keeps the age of its attack.
Multiple sounding voices are retained for independent release and bends even when they contribute one shared pitch reference.

Candidate generation takes the union of Manhattan-radius balls around every contributing context node.
The radius is a hard limit on distance from the nearest reference, not a weighted mean or a box around the original C.
Only selected axes may be traversed; reducing the allowed axes does not rewrite an existing reference's coordinates.
Zero-weight released entries do not generate candidates.
With no contributing context, an explicit origin reference is used.

This finite union moves with the context and can contain separated regions when context itself is widely spread.
That is an explicit prototype choice to evaluate, not a claim that there is always one compact cluster or fewer than twenty reachable nodes.

### Selection

Let input pitch be `p` and the moving reference displacement be `d`.
Realize each eligible node in the octave nearest `p + d`, giving candidate output `q`.
Its score is:

```text
pitch cost   = ((q - p - d) / pitchScale)^2
register_i   = floor + (1 - floor) × exp(-falloff × abs(q - onset_i) / 1200)
weight_i     = contextBaseWeight_i × register_i
harmony cost = harmonic × sum(weight_i × latticeDistance(node, node_i)) / sum(weight_i)
total        = pitch cost + harmony cost
```

Lowest total wins, with deterministic coordinate-order ties.
The finite octave realization and the nonzero pitch term preserve input register relative to the moving frame.
The register floor prevents a distant context note's influence from disappearing entirely.
Whole-performance octave transposition preserves the score and relative outcomes.

After each onset, set `d = q - p`, carrying the full correction without octave wrapping.
This is intentionally the simplest moving reference to probe: it follows the last onset's correction, not a twelve-key map, inferred chord root or long-term origin average.
Order dependence is expected, and changing the last note of a phrase can affect what happens next.
The “intentional Pythagorean E” example works through ordinary pitch error and a different declared harmonic weight, not a separate hard match or chord exception.
There is no automatic retuning and no special-case branch for any musical fixture.

The baseline uses harmonic weight 6, pitch scale 20 cents, half-life 1 second, new-note factor 0.7, released weight 0.1, register floor 0.4 and falloff 0.8 per octave.
The paired precision examples use harmonic weight 2 with the same explicit seeded context and zero initial displacement.
All other fixtures use the baseline.

### Bends, resets and memory

The adaptive correction of a sounding note is frozen.
A player's subsequent bend is added to its audible output but does not change the onset pitch used by the policy.
The bend field is recorded in event history and exported experiments.

Silence means no held voices in this simulator; virtual elapsed time then counts toward the timeout.
Zero timeout means no automatic silence reset, and merely waiting otherwise does not decay memory.
A stop releases all voices and optionally clears released memory and displacement.
A loop optionally clears released memory and displacement while leaving existing held notes and their frozen corrections intact.
Start does not itself reset.
The explicit **Reset context** action releases all voices and clears memory and displacement.
Pedals and real DAW transport semantics are not implemented here; these choices are documented hypotheses for the standalone tool.

## Reachability

The lattice distinguishes eligible nodes from nodes that can actually win for some input.
The input-range controls define the absolute register span being inspected, initially C2 through C7.
The reported set is the union across that entire range and all seventh layers, not just the displayed layer or a single octave.
The range is limited to ten octaves per calculation; this is an inspection bound, not a limit on accumulated drift.
It makes no claim about inputs outside the shown range.

Each candidate/octave realization has constant harmonic cost because its register weights use its realized output `q`.
Its pitch-cost parabola has the same curvature as every competitor's.
Within each nearest-octave validity interval, pairwise score differences are linear, so winning ranges can be computed by clipping intervals at their analytic intersections.
Boundary-only winners are also evaluated explicitly using the selector's tie behaviour.
Calculations use JavaScript floating-point arithmetic.

The result is not a coarse pitch sweep: narrow winning intervals count, and selected-node inspection shows their input boundaries.
A worker performs the calculation so large 7-limit neighbourhoods do not block controls; changing the experiment cancels obsolete work.
Results can take seconds for many widely separated references and a large radius.
No context or candidate set is silently truncated to make that work cheaper.

## Reproducible experiments

The built-in examples distinguish actual sequenced attacks from exact seeded starting pitches.
Seeded fixtures say so explicitly and show those notes in the context table.
They isolate policy comparisons without pretending the algorithm itself produced the starting pitches.
The fifth-chain and drift journeys use actual assignments, including their resulting moving reference updates.

**Edit the sequence** accepts one event per line:

```text
on C3 c
on E3 e
bend e 20
off e
wait 3
on E5+7.82c high
off *
loop
stop
start
reset
```

Pitch strings may use sharps or flats; event IDs name note lifetimes.
`//` begins a comment.
Editing a built-in sequence retains its explicitly seeded starting context; **Start empty** removes that seed.
**Export experiment** saves the parameters, seed, complete event sequence, current cursor and reachability range as JSON.
**Import experiment** restores those inputs rather than trusting serialized derived results.
Browser state is not otherwise persisted automatically.

## Verification

```sh
node --test tools/adaptive-tuning-simulator/model.test.mjs
```

The suite checks the fifteen musical fixtures with declared profiles, 101 full major-third cycles with over three octaves of unwrapped drift, octave transposition, release-recency replacement, repeated pitches, frozen corrections and player bends, reset settings, axis eligibility and analytic reachability against direct selection.
The browser also offers **Musical checks** for running the fifteen fixture outcomes interactively.
Changing parameters may produce a different result and the current example reports that difference rather than declaring every run a pass.

This verifies the model's mechanics and the stated fixtures, not perceptual quality or suitability for every progression.
The main open choices remain the last-onset reference, union-shaped neighbourhood, scoring behaviour away from the examples, and replacing fixed-capacity memory with a better musical rule.
Changing display tolerance is not wired to musical intent; precision currently acts entirely through the pitch-cost term.
The prototype models just tuning only, not the plugin's arbitrary tuning axes or temperament locks, and has no MIDI-device or DAW integration.
