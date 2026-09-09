# Adaptive tuning: a moving harmonic neighbourhood

## Status and scope

Design discussion recorded on 2026-09-09 for Yan and future sessions.
This document records requirements and proposals, not implemented behaviour or a settled algorithm.
Yan explicitly asked to refine the design before implementation.
Continue the discussion and update this document as decisions are made.
The current implementation is described in [adaptive-tuning.md](adaptive-tuning.md).

## Musical objective

Allow indefinite travel through a just-intonation lattice using locally sensible harmonic steps, even when the controller repeats a fixed set of pitches.
There must be no automatic pull toward the original tuning reference and no fixed absolute lattice box that eventually stops the journey.

The motivating progression is repeated C major → E major → A♭ major → C major on a 12-TET keyboard.
With a just lattice, harmonic continuity should allow this to become C major → E major → G♯ major → B♯ major → …,
translating the major-triad shape one step right along the major-third axis at each transition.
The shared tones E, G♯ and B♯ provide a musical route for those translations.

Three just major thirds have ratio `(5/4)^3 = 125/64`, less than an octave.
Relative to the repeated keyboard register, each three-transition cycle therefore accumulates one diesis of downward pitch drift: `1200 log2(128/125) ≈ 41.059 cents`.
Rightward lattice movement and downward accumulated tuning displacement are both intended.
Returning to the same input chord must not reset its lattice position or wrap away the accumulated displacement.

This example is a requirement for general harmonic heuristics, not a request to recognize and special-case these three chords.
MIDI alone cannot distinguish an ordinary return to C from continuing enharmonically to B♯;
the desired musical convention favours continuity along the lattice.

## Requirements established in discussion

### Continuous input pitch

Do not model the input as twelve keyboard pitch classes mapped to twelve lattice nodes.
Any arbitrary pitch can arrive, including from a controller with fine divisions of the octave.
Finer input tuning should give the player more control over nearby harmonic alternatives.
The concrete example is choosing between the minor sevenths 16/9 (approximately 996.09 cents) and 9/5 (approximately 1017.60 cents) above a reference.
When both are harmonically admissible, a more precise input should help select the intended one.

### Context and memory

Released notes must continue to contribute to harmonic context, favouring more recently released notes.
Forget old contributions when they cease to be useful, using straightforward bounded memory rather than an elaborate analysis of whether a note could ever affect a future choice.
The exact weighting and forgetting rules remain open.

The working proposal gives held notes the strongest influence and decreases released-note influence with age.
Forgetting individual old notes must not itself erase the accumulated tuning displacement.
Ordinary releases between chords must not destroy continuity.

Provide a configurable time before reset after silence, with a proposed “Never” option.
The working proposal measures silence from the release of the last sounding note and clears both harmonic memory and accumulated drift at timeout.
Exact pedal, transport and reset behaviour, time range and defaults remain to be decided.

### Hard harmonic boundary

Determine a finite eligible neighbourhood from harmonic context before considering the incoming pitch.
Distance means harmonic distance along the lattice, not acoustic distance in cents.
A sufficiently distant lattice node is ineligible even if it perfectly matches the input pitch.
The incoming pitch must not expand the boundary to reach such a node.

The neighbourhood can move as context changes, allowing unlimited cumulative travel through individually local steps.
The distance metric, treatment of multiple context notes and boundary size are not yet chosen.
In particular, a simple radius of two fifth/major-third steps around a lone root admits 16/9 but excludes 9/5, which requires two fifth steps and one reverse major-third step.
Check the desired harmonic vocabulary before choosing a radius or node-count limit.

### Preference within the boundary

Among eligible candidates, balance closeness to the incoming pitch against harmonic distance or preference.
Yan requested a setting controlling their relative importance.
Pitch fidelity must be measured relative to a moving tuning reference so that accumulated drift does not turn fidelity into an attraction back to the original pitch.
How that reference is represented and updated remains a central open question.

The proposed balance is “Harmonic preference ↔ Pitch fidelity”.
Stronger harmonic preference should make a coarse controller useful for following just relationships;
stronger pitch fidelity should give a finely tuned controller more explicit choice.
Neither end can admit nodes outside the hard boundary.
The proposal retains meaningful pitch sensitivity even at the strongest harmonic setting, since ignoring pitch entirely could make every input choose the same node.
No score formula, weights or slider endpoints have been agreed.

### Lattice reachability indicator

Show the nodes that can actually be selected by any arbitrary next input pitch with the present context held fixed.
Distinguish eligible candidates from reachable winners: an eligible node may never win for any input pitch.
The desired indicator shows the latter set.
It does not show everything reachable after arbitrary future sequences, which can extend indefinitely.

Yan imagines a reasonably small reachable set, fewer than twenty nodes.
Treat that as a design target to validate, not an agreed hard cap that may silently remove useful interval choices.
Sweeping continuous input through one octave is a proposed definition if octave equivalence holds;
register-dependent behaviour has not yet been settled.

Proposed presentation: subtle outlines on reachable nodes, distinct from the appearance of sounding notes.
A faint connecting region and an optional view of each node's input-pitch range are possibilities, not approved UI specifications.
Context and balance changes may move those ranges and make nodes enter or leave the reachable set.

## Proposed controls

| Control | Intended purpose | Status |
|---|---|---|
| Neighbourhood size | Set the maximum permitted harmonic remoteness | Proposed; metric, range and default open |
| Harmonic preference ↔ Pitch fidelity | Balance eligible candidates' harmonic suitability and input-pitch closeness | Requested; scoring, range and default open |
| Reset after silence | Set when context and accumulated drift reset, optionally never | Requested; timing semantics, range and default open |

Memory decay might need a separate control or might use a fixed rule.
It must not be conflated with the silence reset: one lets new harmony take over, while the other intentionally ends the journey.

## Open decisions for the next discussion

1. **Held pitches and anchors.** Must already sounding notes remain fixed, as they do today? How strongly should a sustained bass or pedal constrain travel when upper voices move elsewhere?
2. **Chord timing.** Should simultaneous new notes be selected jointly, and should slightly spread attacks or arpeggios receive the same interpretation? What latency, if any, is acceptable for grouping them?
3. **Moving reference.** What establishes and advances the input-to-lattice reference without causing unintended shifts from chord voicing, attack order or expression? How are register and accumulated unwrapped displacement carried separately?
4. **Harmonic distance.** Which lattice connections and prime axes count, with what weights? Is remoteness measured from one centre, the whole context or nearby individual voices? How is a widely spread context handled without admitting arbitrary bridges or leaving no candidates?
5. **Memory.** How quickly should released harmony lose influence? Should a newer occurrence replace an older contribution, and should sustained notes, chord density or repeated attacks affect weighting?
6. **Reset.** What counts as silence under sustain? What should Stop, playback loops and explicit Reset do? What timeout range and default feel useful?
7. **Continuous pitch and expression.** Which incoming pitch controls describe the pitch to quantize at attack? Should later bends stay expressive, feed the harmonic context, or cause reselection? The current participating path does not use incoming expression to choose its assignment, so continuous input requires an explicit change to that contract.
8. **Selection and indicator.** How strong can pitch fidelity become within the boundary, and how are ties resolved stably? If chord notes are selected jointly, does the indicator describe single-note next actions only or also possible chord outcomes?

Joint chord selection was suggested to avoid keyboard-order and inversion artefacts, but it is not settled.
A fixed twelve-node keyboard mapping was explicitly rejected in favour of continuous pitch input.
Do not restore that assumption merely to simplify grouping or the indicator.

## Behaviour to validate when an algorithm is proposed

- Repeated ascending-third major chords continue rightward for many cycles, beyond one diesis and beyond the old absolute pitch-correction window.
- Ordinary gaps between chords preserve the journey until the configured reset.
- Fine pitch input can distinguish 9/5 and 16/9 when both are eligible.
- An exact pitch match outside the harmonic boundary never wins.
- Reachable nodes shown in the indicator agree with actual next-note selection under the same context.
- Inversions, octave changes, attack order, sustained anchors and arpeggios have deliberate, stated behaviour rather than accidental consequences of sequencing.

These are design checks, not claims of implemented or tested behaviour.
The existing policy's fixed origin domain, 50-cent candidate window and per-note sequential decisions are useful comparison points, not constraints on this redesign.
