# Moving-neighborhood plugin policy

This is the implementation record for policy version 2, based on the [design](adaptive-tuning-design.md) and [simulator](../tools/adaptive-tuning-simulator/README.md).
It replaces the fixed origin domain, 50-cent input window and per-key history.
The existing central sequencer still orders attacks; this change does not introduce chord batching, extra latency, or automatic retuning.

## Musical decisions

The Rust scorer uses the simulator's baseline settings and score.
Input is an absolute pitch, including its register, per-note attack expression and original MIDI channel displacement.
The previous onset's full output-minus-input correction establishes the moving reference.
That displacement is never octave-wrapped, so repeating the same controller chords can continue moving along the lattice.

Candidate nodes are the union of local Manhattan balls around the contributing context.
The selected axes restrict which steps may be taken.
Harmonically remote nodes are excluded before pitch matching, even if their acoustic pitch would match exactly.
The nearest octave realization of each eligible node is scored with pitch error and register-weighted harmonic distance.
Ties use coordinate order.

Every reference decays on one clock:
it halves for every half-life, one second by default, between its attack and the newest attack in context.
A held reference starts at weight one and a released one at the released-to-held weight, 0.1 by default;
a release does not restart the age, so letting go of a note never raises its weight.
Because every weight shares that clock and the score normalizes them, waiting changes no decision,
and a chord's notes, milliseconds apart, weigh alike, which a rank per attack would not give.
A held reference can still weigh less than a release struck well after it:
at the defaults, about 3.3 seconds after.
Repetitions refresh one contribution within the configured absolute-pitch tolerance; they retain separate voice lifetimes for release and expression.
Octaves and comma-shifted returns remain distinct when outside that tolerance.
Released memory holds at most 24 contributions, counted separately from held ones, and a release beyond that evicts the one struck longest ago.
The 24 is a storage bound with no control rather than a musical rule:
every entry decays on the half-life, and at one second the entries beyond the 24th carry under 2% of the weight even at four notes a second.
A larger bound measured costly on the audio thread, since every decision scores each candidate against every entry.
Released memory also fades by count:
each note assigned a lattice node that no held or remembered note occupies multiplies every released contribution's weight by the new-note factor, 0.7 by default.
Repeating a node, in any octave, fades nothing, so a repeated note cannot erase the rest.
This is what lets a fast progression move on:
a major-third cycle played a chord every 0.15 s stalls with 24 remembered notes and no factor, keeps moving at 0.7 and below, and already stalls at 0.75 with no gap between chords.
Retune exclusion clears that source's held and released context, including a moving reference it owned.
A departing source ends its held notes through the session cut;
the resulting released memory follows the ordinary silence, Stop and Reset controls.

Tuning and temperament settings come from the audio owner's resolved configuration.
Custom axis sizes work through the same score, and locked commas canonicalize equivalent coordinates.
Changing settings affects subsequent attacks; the adaptive corrections of existing notes remain fixed.
The lattice's camera reach and display tolerance do not constrain musical travel or determine intent.

## Expression and resets

A note's harmonic reference is its tuned onset pitch.
Later player expression changes emitted pitch but leaves that reference and the adaptive correction unchanged.
Per-note expression is added to the frozen correction.
MIDI channel bend is forwarded and is included separately in attack selection, so it is not counted twice.
The MIDI pitch-bend range defaults to two semitones and follows RPN 0 sensitivity changes.
The participation toggle preserves player pitch while retaining the established note and pedal cleanup.

Scheduled output pitch and onset pitch are separate factual fields.
Channel-pitch changes update the scheduled voices, but no display/take baseline is requested for them, so neither the final nor any intermediate bent pitch reaches the display or take.
That recording limitation is [issue #783](https://github.com/yan-h/harmonigraph/issues/783), closed not-planned;
[the adaptive-tuning record](adaptive-tuning.md#pitch-output) names the code path.

Zero silence timeout means never reset for silence.
A positive timeout is evaluated against the completed input frontier once no held context remains.
Note releases supply their original input sample.
Stop releases voices and, if enabled, clears released memory and displacement.
With stop reset disabled, the ended notes remain recent context.

Loop/seek detection compares the host's seconds timeline with elapsed steady sample time.
A discontinuity greater than two milliseconds marks a pending reset.
With loop reset enabled, the next participating attack clears released memory and displacement;
that attack consumes the pending flag.
Reactivation clears the previous timeline observation.
Held notes retain their frozen assignments.
The host must supply a seconds timeline for this detection.
An explicit session reset clears musical context through the existing reset boundary.

Pedal-aware harmonic holding remains deferred: harmonic held/released status follows the existing note-lifetime release semantics.
Instrument release tails and pedal sustain do not turn released context back into a held contribution.

## Keyboard tuning

The keyboard tuning is three sizes, for primes 3, 5 and 7, describing how the player's controller renders a lattice node.
It shares the lattice's C offset rather than having one of its own.
Scoring obeys one rule with it:
a key may only become a node that the keyboard's own tuning would render at the pitch the key sent.
A candidate is admissible when its keyboard rendering, reduced to one octave, lies within 5¢ of the input pitch class,
and the ordinary pitch and harmonic terms choose among the admissible candidates.
If none is admissible, the attack was bent off every key, and every candidate competes exactly as it would with no keyboard at all.
The window is a fixed 5¢ rather than the same-note tolerance.
A learned fifth multiplied out to twelve or fourteen fifths can miss by a few cents, and a controller that sends its tuning as pitch bend quantizes it;
the window only has to stay well under the smallest distinction a meantone or schismatic keyboard makes, about 20¢.
A deliberate attack bend the size of the 21.5¢ intentional Pythagorean E still falls back.

The rendering is taken from the candidate as it was respelled for the lattice's tempered set, so a tempered lattice needs no special case.
The default is 12-TET (700, 400, 1000¢), which cannot tell apart any two nodes in one semitone class, so the default scorer's drift is unchanged.
What it does remove is a key played in another semitone class:
on the default tempered 12-TET lattice, a key whose nearest same-class node was five steps away used to sound a semitone off, because 100¢ of pitch error cost less than the distance.
A meantone keyboard has a separate B♯ key, so its C key never becomes the B♯ a diesis below;
it cannot tell the 5-limit A from the Pythagorean one, so context chooses between them.
A schismatic keyboard has two A keys, and each becomes its own A.

Learn fills it.
Whenever Learn hears a fifth, the keyboard tuning becomes the one that fifth generates:
5 and 7 sit at the positions on the chain of fifths, within fourteen either way of C, nearest a just 5/4 and a just 7/4, with ties going to the shorter chain.
The learned third and seventh are not used, because on a Pythagorean-side keyboard Learn's third is whichever pressed key sits nearest a just third, which spells prime 5 wrong until the other key has been pressed.
The Tuning pane shows the three sizes, still editable, in a collapsed group under the Learn toggle,
with a **Derive from fifth** button that does from the fifth shown what Learn does from the fifth it hears.

The lattice axes belong to Learn only while no source has Retune on.
With retuning off, the lattice is a picture of the input and should equal the keyboard;
with it on, the lattice is the target, and Learn moves only the shared C offset and the keyboard tuning.
Toggling Retune copies nothing in either direction.

The live neighborhood outlines apply the same filter.
Besides the analytic boundaries, the worker plays every key the candidates render, in each octave of the C2–C7 range.
The simulator has no keyboard tuning, so with any keyboard but 12-TET the outline is Rust-only.

Cases written down rather than handled:

1. The outlines are approximate inside a key's tolerance window.
They play each key at its exact rendering, so a node that wins only between that pitch and the window's edge is missed;
and the analytic envelope still lists an unfiltered winner whose whole winning range lies inside key windows, where the filter always overrides it.
The windows are 10¢ wide, so this matters only where two admissible nodes nearly tie.
2. A policy edit from the pane sends the whole policy the editor last saw, keyboard included.
One that lands just after Learn has changed the keyboard puts the old keyboard back until the held chord next changes and Learn fires again.
3. When no node of the key's class is within the neighbourhood, as with a small radius on a tempered lattice, the fallback still plays the key at the nearest node's pitch, up to a semitone off, as it always has.

An accepted consequence, not a bug:
the syntonic comma pump still drifts on a meantone keyboard, because that keyboard cannot tell `(1,0)` from `(-3,1)`.
It is the same comma as the one between the fifths-chain A and the 5-limit A, which is why no pin strength could separate the two;
see [issue #852](https://github.com/yan-h/harmonigraph/issues/852).

## Controls and live neighborhood

The Tuning pane exposes the keyboard tuning, harmonic weight, pitch scale, neighborhood radius, allowed axes, half-life, new-note factor, released-to-held weight, register weight per octave, same-note tolerance, silence timeout and transport reset choices.
Defaults match the simulator's baseline profile.
The precision profile used by the paired intentional-E examples is obtained by setting harmonic weight to two.

The live lattice can outline nodes that win for some arbitrary next input in the explicitly labeled C2–C7 register range.
This is a union across registers and seventh layers, not twelve keyboard mappings or a single-octave sample.
The count can exceed twenty depending on context and settings.
Only nodes in the current camera window receive visible outlines; the count includes off-screen winners.
The outlines are selection-style UI annotations and can overlap foreground geometry.
They are intentionally absent from preview/export pictures, like live hover and session controls.

The Hub publishes its authoritative next-attack context, including scheduled predecessor assignments, rather than reconstructing it from visual note fades.
A worker calculates winner intervals analytically and checks their boundaries.
Changing context cancels obsolete work and hides its old result while the new result is pending.
The cache key contains only resolved musical settings, onset references, weights and unwrapped displacement.
Camera changes, callback time, performance counters and display fades do not restart that calculation.

## Resource and persistence contracts

Selection uses preallocated scratch and an explicit ceiling of 4096 distinct candidates, up to 256 held voices and 24 released contributions.
Exceeding the candidate ceiling refuses that evaluation;
the onset receives no adaptive correction and the policy fault is reported.
It never silently truncates the neighborhood or chooses from a partial set.
This ceiling bounds individual evaluation work, not the total distance of a musical journey.
Large onset cohorts can exceed the audio-callback budget well below the ceiling;
[issue #790](https://github.com/yan-h/harmonigraph/issues/790) tracks that measurement.
Tune's output delay and missed-correction counter do not bound the Hub's processing time.
The substantially more expensive winner-range calculation runs outside the audio callback.

Coordinates travel as signed 32-bit lattice coordinates and correction as signed 64-bit microcents.
They no longer have the former eight-bit coordinate or approximately 2147-cent correction limits.
Machine coordinate exhaustion is reported explicitly.

Adaptive settings are saved in musical settings and copied into assignment/take configuration metadata.
Missing fields use the baseline defaults and values are sanitized identically for UI preview, audio adoption and saving.
Old policy descriptor fields are removed, and old takes missing the new settings use baseline metadata defaults on replay.
Recorded output pitches continue to drive replay; replay does not rerun the adaptive algorithm.
Onset pitch is a new saved voice field.
No persisted compatibility aliases or migration shims are added.

## Validation

The core parity fixtures are generated from the simulator with:

```sh
node tools/adaptive-tuning-simulator/export-plugin-fixtures.mjs
```

They record every onset in all fifteen musical examples with their declared settings and seeded context.
Rust and JavaScript differ only in axis precision: the plugin uses fixed microcents and the simulator uses floating-point logarithms.
Parity compares coordinates exactly and output pitches within 0.001 cent.

The production tests exercise the exported CLAP path, including 304 major chords on a repeating controller pattern, post-attack expression, frozen context, configuration boundaries, memory replacement and reset settings.
The 304-chord journey crosses both the old coordinate and correction representation limits.
The existing sequencing, ownership and accepted-output tests remain the transport contract.
