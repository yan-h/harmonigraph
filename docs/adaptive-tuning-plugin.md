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

Held references have weight one.
Released references use the configured released weight multiplied by the release carry-over factor to the power of their recency rank.
Repetitions refresh one contribution within the configured absolute-pitch tolerance; they retain separate voice lifetimes for release and expression.
Octaves and comma-shifted returns remain distinct when outside that tolerance.
The temporary released-memory budget counts released contributions separately from held ones.
New activity replaces memory; waiting does not gradually decay it.
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

## Controls and live neighborhood

The Tuning pane exposes harmonic weight, pitch scale, neighborhood radius, allowed axes, recent-memory capacity, released-note weighting, register weighting, same-note tolerance, silence timeout and transport reset choices.
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
