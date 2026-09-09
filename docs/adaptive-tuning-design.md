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

### Context determines the minor-seventh preference

The 9/5 versus 16/9 example is not solely a test of a finely tuned controller selecting either interval.
Yan specified the following contextual expectations for a subsequent B♭, with ratios measured from C:

| Existing context | Expected B♭ | Musical relationship |
|---|---|---|
| C–E♭–G | 9/5 | With E♭ at 6/5, B♭ at 9/5 is a just fifth above E♭ |
| C–E–G | No fixed answer | Yan is unsure and accepts that algorithm parameters may decide |
| C–F | 16/9 | With F at 4/3, B♭ at 16/9 is a just fourth above F, equivalently a fifth below it modulo octaves |

Use these as examples of context-dependent selection, not hard-coded chord-recognition rules.
Compare the contexts with the same incoming B♭ pitch to isolate the effect of harmony from finer controller input.
The ratio explanations assume just context pitches; a fixture must establish their actual tuned onset pitches rather than infer them from keyboard labels alone.
In particular, the C–E♭–G expectation uses E♭ at 6/5, and the C–F expectation uses F at 4/3.
These examples constrain the harmonic metric alongside the ascending-third travel requirement.
Fine input control remains useful, but does not replace the requirement for harmony to influence the choice.

### Context and memory

Released notes must continue to contribute to harmonic context, favouring more recently released notes.
Forget old contributions when they cease to be useful, using straightforward bounded memory rather than an elaborate analysis of whether a note could ever affect a future choice.
**New musical activity replaces old context; elapsed time alone does not gradually weaken it.** If Yan plays and leaves a note, later notes must still use that context for as long as the configured memory/reset setting permits.
The exact replacement and weighting rules remain open.

Repeating a note refreshes its existing contribution rather than accumulating copies or increasing its weight merely through repetition.
If the same keyboard C returns one diesis lower, it is an entirely separate new pitch and must be treated as such.
Identity follows the resulting pitch/lattice placement, not controller-key identity.
Yan suggested a tolerance setting for deciding when a new note counts as the same as an old one.
The proposed tolerance concerns memory matching, not permission to retune a sounding note.
Its comparison domain, units, default, interaction with lattice spelling and eventual register-sensitive matching still need to be specified.
Do not choose a default that conflates the motivating diesis-separated pitches or the desired minor-seventh alternatives.

For the initial version, Yan accepts remembering the last N distinct contributions, refreshing repeats and evicting the least recently used released contribution when capacity is exceeded.
Held notes remain represented rather than being evicted to meet the remembered-note budget.
The capacity may be configurable; its value and accounting for held versus released entries remain open.
**This is explicitly a temporary simplification.** Yan wants to move away from this fixed recent-note scheme eventually, toward a more musically informed replacement policy.
Do not turn the initial capacity rule into a permanent musical contract or implement the more elaborate successor before it is designed.

Giving held notes stronger influence remains a working proposal, not an absolute anchoring rule.
A sustained bass may keep the neighbourhood nearby or be left behind by the newer harmony, depending on the sequence of notes and harmonic heuristics.
Yan accepts either outcome; do not impose a universal pedal anchor or universal escape rule.
Forgetting individual old notes must not itself erase the accumulated tuning displacement.
Ordinary releases between chords must not destroy continuity.

Octave placement should influence the final algorithm, but Yan explicitly accepts ignoring it in the initial version for simplicity.
The eventual harmonic context must store notes by register, and evaluate an incoming note using its register relative to those context notes.
This is more than giving bass notes a fixed extra weight: relationships between input and context registers belong in the eventual decision.
The exact register-sensitive heuristic is deferred, not a requirement to introduce it in the first version.
Do not declare permanent octave equivalence or make a pitch-class-only representation the final context model.
Output register and accumulated tuning displacement still need to be preserved even while initial harmonic scoring ignores register.

Provide a configurable time before reset after silence, with a proposed “Never” option.
The working proposal measures silence from the release of the last sounding note and clears both harmonic memory and accumulated drift at timeout.
Whether transport events reset the context must be configurable, rather than always resetting or always preserving it.
This answers the discussion of stopping/restarting playback and looping;
the exact event triggers, whether they share one control or have separate controls, and defaults remain open.
Exact pedal and reset behaviour, timeout range and the treatment of accumulated drift under transport reset still need to be specified.

### Frozen notes and sequential decisions

**No adaptive retuning of an already sounding note, ever, under the present design.** Yan explicitly made this an immutable rule unless he later changes his mind.
Choose the adaptive tuning correction at attack and retain it for the note's lifetime, regardless of subsequent harmony or neighbourhood movement.
Do not reselect or glide a held note to improve a later chord.
Player-authored pitch bends after attack must remain audible, composed with the fixed adaptive correction.
The no-retuning rule forbids automatic changes by the algorithm, not expressive bends by the player.

For the initial version, harmonic context uses each note's tuned onset pitch and does not follow its subsequent bends.
This is an intentional simplification: a bent note's current audible pitch may differ from the pitch contributing to future tuning decisions.
Whether and how harmonic context should follow player bends remains an open question for later design.
Do not silently implement bend-following context or suppress the player's bends to make those two pitches agree.
Onset-based policy context does not redefine actual emitted-pitch reporting as onset-only reporting.

Prefer a simple algorithm and accept that the route through the notes affects the result.
A chord played together, rolled or arpeggiated need not arrive at the same tuning, and changing note order may change the path.
Do not introduce joint chord optimization or extra grouping latency just to make those cases equivalent.
The implementation still needs a deterministic order for simultaneous attacks.

### Hard harmonic boundary

Determine a finite eligible neighbourhood from harmonic context before considering the incoming pitch.
Distance means harmonic distance along the lattice, not acoustic distance in cents.
A sufficiently distant lattice node is ineligible even if it perfectly matches the input pitch.
The incoming pitch must not expand the boundary to reach such a node.

The neighbourhood can move as context changes, allowing unlimited cumulative travel through individually local steps.
The distance metric, treatment of multiple context notes and boundary size are not yet chosen.
In particular, a simple radius of two fifth/major-third steps around a lone root admits 16/9 but excludes 9/5, which requires two fifth steps and one reverse major-third step.
Check the desired harmonic vocabulary before choosing a radius or node-count limit.

Provide a setting controlling which lattice axes or interval families the algorithm may use.
Yan requested configurability rather than a fixed choice between fifth/major-third relationships and including seventh-based relationships such as 7/4.
The UI shape, available combinations, default and behaviour when that setting changes remain open.
Changing the allowed vocabulary must still respect the prohibition on retuning existing notes.

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
For the initial version, which may ignore register in harmonic decisions, sweeping continuous input through one octave is a proposed definition.
Revisit that definition when the deferred register-sensitive behaviour is designed.

Proposed presentation: subtle outlines on reachable nodes, distinct from the appearance of sounding notes.
A faint connecting region and an optional view of each node's input-pitch range are possibilities, not approved UI specifications.
Context and balance changes may move those ranges and make nodes enter or leave the reachable set.

## Proposed controls

| Control | Intended purpose | Status |
|---|---|---|
| Neighbourhood size | Set the maximum permitted harmonic remoteness | Proposed; metric, range and default open |
| Harmonic preference ↔ Pitch fidelity | Balance eligible candidates' harmonic suitability and input-pitch closeness | Requested; scoring, range and default open |
| Reset after silence | Set when context and accumulated drift reset, optionally never | Requested; timing semantics, range and default open |
| Reset on transport events | Choose whether stopping/restarting playback and looping reset context | Requested; event triggers, one versus separate controls, reset scope and defaults open |
| Allowed lattice axes / interval families | Control which harmonic relationships may supply candidates, including whether seventh-based relationships are allowed | Requested; UI, combinations and default open |
| Same-note tolerance | Decide when a new pitch refreshes an existing memory contribution | Suggested by Yan; matching semantics, units, range and default open |
| Remembered-note capacity | Set N for the temporary recent-note replacement scheme | Scheme accepted for initial version; exposing N as a setting remains proposed |

The initial memory replacement scheme is intentionally temporary; its successor remains to be designed.
Do not add gradual time decay as the default interpretation of forgetting.
Replacement must not be conflated with the silence reset: one lets new harmony take over, while the other intentionally ends the journey.

## Open decisions for the next discussion

1. **Moving reference.** What establishes and advances the input-to-lattice reference? How are register and accumulated unwrapped displacement carried separately? Note-order dependence is accepted, but updates still need a coherent musical interpretation.
2. **Harmonic distance and vocabulary setting.** Which axis combinations should the setting offer, and how are permitted connections weighted? Is remoteness measured from one centre, the whole context or nearby individual voices? How is a widely spread context handled without admitting arbitrary bridges or leaving no candidates?
3. **Memory matching and weights.** Different resulting pitches remain distinct even when played by the same key. How should the proposed same-note tolerance work, and what is N for the accepted temporary recent-note scheme? Should sustained notes or chord density affect weighting? A held bass has no categorical anchoring privilege; its effect should emerge from the sequence and heuristics. The eventual successor to fixed-capacity recency remains open.
4. **Reset.** What counts as silence under sustain? Which transport transitions trigger the configurable reset, and should stop/start and loop reset share a control? Does a transport reset clear accumulated displacement along with context? What timeout range and defaults feel useful?
5. **Continuous pitch and expression.** Which incoming pitch controls describe the pitch to quantize at attack? Post-attack player bends remain audible, while the initial policy context retains tuned onset pitch. Bend-aware context is deferred; automatic reselection of sounding notes is forbidden. The current participating path does not use incoming expression to choose its assignment, so continuous input requires an explicit change to that contract.
6. **Selection and indicator.** How strong can pitch fidelity become within the boundary, and how are ties and simultaneous attacks ordered stably? Later, how should the incoming note's register relative to register-bearing context influence selection, and how should the indicator express register-dependent reachability?

Joint chord selection was an earlier suggestion; the subsequent decision favours simplicity and accepts route dependence.
A fixed twelve-node keyboard mapping was explicitly rejected in favour of continuous pitch input.
Do not restore that assumption merely to simplify the indicator.

## Behaviour to validate when an algorithm is proposed

- Repeated ascending-third major chords continue rightward for many cycles, beyond one diesis and beyond the old absolute pitch-correction window.
- Ordinary gaps between chords preserve the journey until the configured reset.
- A lone remembered note remains useful after waiting, provided the silence timeout has not expired; new musical activity, rather than gradual time decay, replaces its influence.
- Repetition refreshes one remembered contribution without multiplying its weight by the number of attacks.
- Replaying a keyboard pitch at a diesis-shifted output creates a distinct contribution; any memory tolerance must have deliberately stated matching behaviour.
- In the temporary N-entry scheme, new distinct activity evicts the least recently used released contribution, while held notes remain represented.
- Later notes never cause an already sounding note to be adaptively retuned.
- Player bends after attack remain audible with the adaptive correction fixed; the initial harmonic context continues to use tuned onset pitch.
- Transport-triggered context reset follows its setting rather than an unconditional preserve/reset policy.
- Fine pitch input can distinguish 9/5 and 16/9 when both are eligible.
- With the same subsequent B♭ input, just C–E♭–G context favours 9/5 and just C–F context favours 16/9; C–E–G has no prescribed answer and may depend on parameters.
- An exact pitch match outside the harmonic boundary never wins.
- The allowed-vocabulary setting controls candidate generation without retuning notes already sounding.
- Reachable nodes shown in the indicator agree with actual next-note selection under the same context.
- Note-order dependence is accepted for inversions and arpeggios; simultaneous attacks must still have a deterministic order, sustained-anchor behaviour needs explicit heuristics, and register weighting may be deferred initially.
- The eventual register-sensitive version stores context notes by register and evaluates incoming notes in relation to those registers; this is deferred rather than claimed by the initial version.

These are design checks, not claims of implemented or tested behaviour.
The existing policy's fixed origin domain, 50-cent candidate window and per-note sequential decisions are useful comparison points, not constraints on this redesign.
