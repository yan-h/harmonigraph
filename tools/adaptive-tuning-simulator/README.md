# Adaptive tuning reference model

A headless JavaScript reference implementation of the [moving-neighbourhood design](../../docs/adaptive-tuning-design.md).
It runs independently of the Rust plugin and does not install or load a DAW build.
The model is a testable first hypothesis, not the final adaptive policy.
The interactive browser laboratory that once sat on top of this model was retired in [#975](https://github.com/yan-h/harmonigraph/issues/975); this directory now holds only the model, its worked examples, its Node test suite and the plugin-fixture exporter.
There are no package installs, CDN resources, telemetry or external service dependencies.

## Run

```sh
node --test tools/adaptive-tuning-simulator/model.test.mjs
```

Regenerate the Rust policy parity fixtures after an intentional model change with:

```sh
node tools/adaptive-tuning-simulator/export-plugin-fixtures.mjs
```

That command writes `crates/harmonigraph-core/src/policy/fixtures.txt`, which the Rust policy tests `include_str!` directly; run `cargo test -p harmonigraph-core` afterward to confirm parity.

## The model

Coordinates are `[fifths, major thirds, harmonic sevenths]`, with just axis sizes `1200 log2(3/2)`, `1200 log2(5/4)` and `1200 log2(7/4)`.
C3 at 4800 cents is coordinate `[0,0,0]`.
Absolute output register is stored separately from lattice coordinates.

### Context and eligibility

A released note never outranks a held one, however long that has been held:
while anything is held, the context is the held notes alone, and released memory is the context only when nothing is held.
Every contribution decays on one clock.
Let `newest` be the latest attack in context and `t` a contribution's own attack.
Its base weight is `0.5^((newest - t) / halfLife)`, with no time decay when the half-life is zero.
A strike on the node struck last, with nothing else struck between, is a hold:
its `t` stays that earlier strike's, so repeating a note, in any octave, moves no clock and weighs exactly as holding it.
Released memory fades by time alone, so a progression keeps moving when its chords are at least about one half-life apart;
closer than that, the chords before pull each new root back.
Every weight shares that clock and the score normalizes them, so waiting changes no decision and a chord's simultaneous onsets weigh alike.
A release does not restart a note's age, so memory is ordered by attack, not by release.
Released memory keeps the 24 released entries struck most recently, counted separately from held voices, which are never evicted to make room.
The 24 is a fixed storage bound with no control: even at a one-second half-life, entries beyond it carry under 2% of the weight at four notes a second.

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
pitch cost   = expm1(((q - p - d) / pitchFlexibility)^2) / expm1(1)
register_i   = perOctave ^ (abs(q - onset_i) / 1200)
vote_n       = max over the voices i on node n of contextBaseWeight_i × register_i
distance     = sum(vote_n × latticeDistance(node, n)) / sum(vote_n)
benefit      = 1 / (1 + distance)
total        = pitch cost - benefit
```

Lowest total wins, with deterministic coordinate-order ties.
Keeping the incoming pitch plus drift scores zero; a node must earn a strictly negative total to beat it.
Unsnapped notes have no assigned node, and their actual pitches are projected locally only for subsequent context scoring.
The finite octave realization and the nonzero pitch term preserve input register relative to the moving frame.
Each octave apart multiplies a context note's vote by the same factor, so a distant note's influence fades smoothly and never reaches zero.
A node sounding in several registers votes once, through its strongest voice, so an octave doubling adds nothing.
Whole-performance octave transposition preserves the score and relative outcomes.

After each assigned onset, set `d = q - p`, carrying the full correction without octave wrapping.
This is intentionally the simplest moving reference to probe: it follows the last assigned onset's correction, not a twelve-key map, inferred chord root or long-term origin average.
Order dependence is expected, and changing the last note of a phrase can affect what happens next.
The “intentional Pythagorean E” example works through exponential pitch error and a different declared pitch flexibility, not a separate hard match or chord exception.
There is no automatic retuning and no special-case branch for any musical fixture.

The baseline uses pitch flexibility 100 cents, half-life 0.5 seconds and register weight 0.7 per octave.
The paired precision examples use pitch flexibility 50 cents with the same explicit seeded context and zero initial displacement.
The flexibility replaces both old scoring controls; it is the displacement where pitch cost equals the maximum possible harmonic benefit, so an entire flexibility-unit move cannot beat staying unsnapped.
The fifths-only example cuts the lattice to one axis and radius 1, which is the profile in which an onset can fall outside every node's window and stay unsnapped.
All other fixtures use the baseline.

### Bends, resets and memory

The adaptive correction of a sounding note is frozen.
A player's subsequent bend is added to its audible output but does not change the onset pitch used by the policy.
The bend field is recorded in event history.

Silence means no held voices in this simulator; virtual elapsed time then counts toward the timeout.
Zero timeout means no automatic silence reset, and merely waiting otherwise does not decay memory.
A stop releases all voices and optionally clears released memory and displacement.
A loop optionally clears released memory and displacement while leaving existing held notes and their frozen corrections intact.
Start does not itself reset.
An explicit reset event releases all voices and clears memory and displacement.
Pedals and real DAW transport semantics are not implemented here; these choices are documented hypotheses for the reference model.

## Reachability

`Simulator.reachability` distinguishes eligible nodes from nodes that can actually win for some input.
Its range argument defines the absolute register span being inspected.
The reported set is the union across that entire range and all seventh layers, not just a single octave.
The range is limited to ten octaves per calculation; this is an inspection bound, not a limit on accumulated drift.
It makes no claim about inputs outside the given range.

Each candidate/octave realization has constant harmonic benefit because its register weights use its realized output `q`.
First solve the interval where its pitch cost is smaller than that benefit.
The difference between two shifted exponential pitch curves is strictly monotone, so overlapping intervals have at most one crossing, found by bisection.
Inputs where every node costs more than staying unsnapped leave gaps in the reported ranges.
Boundary-only winners are also evaluated explicitly using the selector's tie behaviour.
Calculations use JavaScript floating-point arithmetic.

The result is not a coarse pitch sweep: narrow winning intervals count, and `model.test.mjs` checks selected-node results against direct selection at their input boundaries.
No context or candidate set is silently truncated to make that work cheaper.

## Worked examples and program text

`examples.mjs` distinguishes actual sequenced attacks from exact seeded starting pitches.
Seeded fixtures say so explicitly and carry those notes in their seed list.
They isolate policy comparisons without pretending the algorithm itself produced the starting pitches.
The fifth-chain and drift journeys use actual assignments, including their resulting moving reference updates.

Each example's `program` is `parseProgram`'s text format, one event per line:

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
`export-plugin-fixtures.mjs` replays every example's program against the model and writes the assigned onsets, in the plugin's own integer units, to the committed Rust fixture.

## Verification

```sh
node --test tools/adaptive-tuning-simulator/model.test.mjs
```

The suite checks the eighteen musical fixtures with declared profiles, 101 full major-third cycles with over three octaves of unwrapped drift, octave transposition, release-recency replacement, repeated pitches, frozen corrections and player bends, reset settings, axis eligibility, an onset no node is worth snapping to, and analytic reachability against direct selection.
Changing parameters may produce a different result and the current example reports that difference rather than declaring every run a pass.
The workspace group in `ci.sh` runs this suite on every pull request.

This verifies the model's mechanics and the stated fixtures, not perceptual quality or suitability for every progression.
The main open choices remain the last-onset reference, union-shaped neighbourhood, scoring behaviour away from the examples, and replacing fixed-capacity memory with a better musical rule.
Changing tolerance is not wired to musical intent; precision currently acts entirely through the pitch-cost term.
The model covers just tuning only, not the plugin's arbitrary tuning axes or temperament locks, and has no MIDI-device or DAW integration.
